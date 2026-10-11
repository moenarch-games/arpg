// Reliable one-shot commands on the dedicated session (#143).
//
// One-shot commands (attacks, interact, weapon and target changes) travel on the
// game-server reliable command lane (game-server ADR 0008); held state stays on datagrams.
// These tests drive the real `DedicatedGameSession` over a fake WebTransport with a fake
// bidirectional stream and a datagram writer.
//
// API contract (web/src/dedicated-session.js):
//
// - `encodeCommandLaneHeader()` returns the six header bytes `GSCL`, lane format version 1
//   and game protocol version 3.
// - `decodeReliableCommandAck(bytes)` decodes exactly one canonical seven-byte ack
//   (version 3, kind 6, big-endian non-zero u32 sequence, outcome 1/2/3) to
//   `{ sequence, outcome }` with outcome "applied" | "duplicate" | "rejected", and throws
//   on any other length, version, kind, zero sequence or outcome.
// - `session.sendReliableCommand(sequence, payload)` sends one command on the lane and
//   returns a promise that settles when the authority acknowledged it: it resolves with
//   "applied" or "duplicate" and rejects with an Error whose message contains "rejected"
//   for a `Rejected` ack. Every outcome releases the command. Until then the command stays
//   buffered (at most 256 unacknowledged commands; more reject with a message containing
//   "256") and survives reconnects: after a resume, the session opens a lane on the new
//   connection and re-sends every unacknowledged command, in order, before newer ones.
//   Reliable sequences must strictly increase within a session (zero and repeats reject);
//   they are independent of datagram sequences, so a reliable command may carry a lower
//   sequence than a datagram sent before it.
// - The lane is the connection's one bidirectional stream from
//   `transport.createBidirectionalStream()`. It is opened lazily: on the first reliable
//   command of a connection, or right after a resume when unacknowledged commands exist.
//   A connection without one-shots opens no stream. The header is written first, then
//   `encodeGameServerCommand` frames in order.
// - Acks may arrive split or batched in any chunks. Each must decode canonically and name
//   the oldest command in flight on that lane. A malformed or unexpected ack, or the ack
//   stream ending while commands are in flight, fails closed: the session closes that
//   transport, reports `onError`, keeps the commands unacknowledged and resumes as it does
//   for any transport failure.
// - `sendCommand` stays the datagram lane and never writes to the stream.
// - `close()`, and a reconnect grace that expires, reject every unacknowledged command.
import { describe, expect, test } from "bun:test";

import {
  DedicatedGameSession,
  decodeReliableCommandAck,
  encodeCommandLaneHeader,
} from "./dedicated-session.js";

const HEADER = [0x47, 0x53, 0x43, 0x4c, 1, 3];
const APPLIED = 1;
const DUPLICATE = 2;
const REJECTED = 3;

function welcomeFrame({ epoch = 1, tokenByte = 0x11, tickHz = 60, graceTicks = 600n } = {}) {
  const frame = new Uint8Array(46);
  const view = new DataView(frame.buffer);
  frame[0] = 3;
  frame[1] = 3;
  view.setUint32(2, 7, false);
  view.setUint16(6, tickHz, false);
  view.setUint16(8, 4, false);
  view.setBigUint64(10, 99n, false);
  view.setUint32(18, epoch, false);
  frame.fill(tokenByte, 22, 38);
  view.setBigUint64(38, BigInt(graceTicks), false);
  return frame;
}

function ackFrame(sequence, outcome, { version = 3, kind = 6 } = {}) {
  const frame = new Uint8Array(7);
  frame[0] = version;
  frame[1] = kind;
  new DataView(frame.buffer).setUint32(2, sequence, false);
  frame[6] = outcome;
  return frame;
}

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

// Settles a promise into an inspectable record without leaving a rejection unhandled.
function track(promise) {
  const record = { settled: false, value: undefined, error: undefined };
  promise.then(
    (value) => Object.assign(record, { settled: true, value }),
    (error) => Object.assign(record, { settled: true, error }),
  );
  return record;
}

async function until(condition, rounds = 200) {
  for (let round = 0; round < rounds && !condition(); round += 1) {
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
  expect(condition()).toBe(true);
}

async function idle(rounds = 20) {
  for (let round = 0; round < rounds; round += 1) {
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
}

// One fake bidirectional stream: records what the client writes and lets the test send
// ack bytes back.
class FakeLane {
  written = [];
  constructor() {
    const lane = this;
    this.readable = new ReadableStream({
      start(controller) {
        lane.controller = controller;
      },
    });
    this.writable = new WritableStream({
      write(chunk) {
        lane.written.push(new Uint8Array(chunk));
      },
    });
  }

  bytes() {
    const total = this.written.reduce((sum, chunk) => sum + chunk.byteLength, 0);
    const bytes = new Uint8Array(total);
    let offset = 0;
    for (const chunk of this.written) {
      bytes.set(chunk, offset);
      offset += chunk.byteLength;
    }
    return bytes;
  }

  // The header and the sequences of every complete command frame written so far.
  frames() {
    const bytes = this.bytes();
    const header = [...bytes.subarray(0, HEADER.length)];
    const sequences = [];
    const payloads = [];
    let offset = HEADER.length;
    while (offset + 8 <= bytes.byteLength) {
      const view = new DataView(bytes.buffer, bytes.byteOffset + offset);
      expect(bytes[offset]).toBe(3);
      expect(bytes[offset + 1]).toBe(1);
      const length = view.getUint16(6, false);
      if (offset + 8 + length > bytes.byteLength) break;
      sequences.push(view.getUint32(2, false));
      payloads.push([...bytes.subarray(offset + 8, offset + 8 + length)]);
      offset += 8 + length;
    }
    return { header, sequences, payloads };
  }

  sequences() {
    return this.frames().sequences;
  }

  ack(...chunks) {
    for (const chunk of chunks) this.controller.enqueue(chunk);
  }

  finish() {
    this.controller.close();
  }

  fail(error) {
    try {
      this.controller.error(error);
    } catch {
      // Already closed.
    }
  }
}

class FakeTransport {
  datagramsSent = [];
  lanes = [];
  closedFlag = false;

  constructor(welcome, { ready = true } = {}) {
    this.readyDeferred = deferred();
    this.ready = this.readyDeferred.promise;
    if (ready) this.readyDeferred.resolve();
    this.closedDeferred = deferred();
    this.closed = this.closedDeferred.promise;
    this.incomingUnidirectionalStreams = new ReadableStream({
      start(controller) {
        controller.enqueue(
          new ReadableStream({
            start(stream) {
              stream.enqueue(welcome);
              stream.close();
            },
          }),
        );
        controller.close();
      },
    });
    const transport = this;
    let datagramController;
    this.datagrams = {
      readable: new ReadableStream({
        start(controller) {
          datagramController = controller;
        },
      }),
      writable: new WritableStream({
        write(frame) {
          transport.datagramsSent.push(new Uint8Array(frame));
        },
      }),
    };
    this.datagramController = datagramController;
  }

  async createBidirectionalStream() {
    if (this.closedFlag) throw new Error("transport is closed");
    const lane = new FakeLane();
    this.lanes.push(lane);
    return lane;
  }

  datagramSequences() {
    return this.datagramsSent.map((frame) =>
      new DataView(frame.buffer, frame.byteOffset).getUint32(2, false),
    );
  }

  releaseReady() {
    this.readyDeferred.resolve();
  }

  drop() {
    if (this.closedFlag) return;
    this.closedFlag = true;
    this.datagramController.close();
    for (const lane of this.lanes) lane.fail(new Error("transport closed"));
    this.closedDeferred.resolve();
  }

  close() {
    this.drop();
  }
}

// A session whose transports are handed out in order; `connected()` counts welcomes.
async function fixture(transports, { callbacks = {}, ...options } = {}) {
  const queue = [...transports];
  const errors = [];
  let welcomes = 0;
  const session = new DedicatedGameSession({
    endpoint: "https://game.example/arpg",
    transportFactory: () => {
      const next = queue.shift();
      if (!next) throw new Error("unexpected transport attempt");
      return next;
    },
    reconnectDelayMs: 1,
    sleep: async () => {},
    ...options,
  });
  await session.connect({
    onWelcome: () => {
      welcomes += 1;
    },
    onError: (error) => errors.push(error),
    ...callbacks,
  });
  return { session, errors, welcomes: () => welcomes };
}

describe("reliable command lane codec", () => {
  test("the lane header is GSCL, lane format 1 and game protocol 3", () => {
    expect([...encodeCommandLaneHeader()]).toEqual(HEADER);
  });

  test("decodes each canonical ack outcome", () => {
    expect(decodeReliableCommandAck(ackFrame(0x01020304, APPLIED))).toEqual({
      sequence: 0x01020304,
      outcome: "applied",
    });
    expect(decodeReliableCommandAck(ackFrame(9, DUPLICATE))).toEqual({
      sequence: 9,
      outcome: "duplicate",
    });
    expect(decodeReliableCommandAck(ackFrame(0xffffffff, REJECTED))).toEqual({
      sequence: 0xffffffff,
      outcome: "rejected",
    });
  });

  test("rejects every non-canonical ack", () => {
    const valid = ackFrame(5, APPLIED);
    const malformed = [
      valid.subarray(0, 6),
      new Uint8Array([...valid, 0]),
      ackFrame(5, APPLIED, { version: 2 }),
      ackFrame(5, APPLIED, { kind: 5 }),
      ackFrame(5, APPLIED, { kind: 1 }),
      ackFrame(0, APPLIED),
      ackFrame(5, 0),
      ackFrame(5, 4),
      ackFrame(5, 0xff),
    ];
    for (const bytes of malformed) {
      expect(() => decodeReliableCommandAck(bytes)).toThrow();
    }
  });
});

describe("dedicated one-shot delivery", () => {
  test("one-shots go on the lane after its header; held commands stay datagrams", async () => {
    const first = new FakeTransport(welcomeFrame());
    const { session } = await fixture([first]);

    await session.sendCommand(1, new Uint8Array([1]));
    expect(first.datagramSequences()).toEqual([1]);
    expect(first.lanes).toHaveLength(0);

    const attack = track(session.sendReliableCommand(2, new Uint8Array([7, 8])));
    await until(() => first.lanes[0]?.sequences().length === 1);
    expect(first.lanes[0].frames()).toEqual({
      header: HEADER,
      sequences: [2],
      payloads: [[7, 8]],
    });
    await session.sendCommand(3, new Uint8Array([3]));
    expect(first.datagramSequences()).toEqual([1, 3]);

    // A second one-shot on the same connection reuses its one lane.
    const interact = track(session.sendReliableCommand(4, new Uint8Array([4])));
    await until(() => first.lanes[0].sequences().length === 2);
    expect(first.lanes).toHaveLength(1);
    expect(first.lanes[0].sequences()).toEqual([2, 4]);

    expect(attack.settled).toBe(false);
    first.lanes[0].ack(ackFrame(2, APPLIED));
    await until(() => attack.settled);
    expect(attack.value).toBe("applied");
    first.lanes[0].ack(ackFrame(4, APPLIED));
    await until(() => interact.settled);
    expect(interact.value).toBe("applied");
    session.close();
  });

  test("a reliable sequence may be lower than a datagram sent before it", async () => {
    const first = new FakeTransport(welcomeFrame());
    const { session } = await fixture([first]);
    await session.sendCommand(5, new Uint8Array([5]));
    const attack = track(session.sendReliableCommand(4, new Uint8Array([4])));
    await until(() => first.lanes[0]?.sequences().length === 1);
    first.lanes[0].ack(ackFrame(4, APPLIED));
    await until(() => attack.settled);
    expect(attack.value).toBe("applied");
    session.close();
  });

  test("one-shots stay buffered until acked, and applied, duplicate and rejected all release them", async () => {
    const first = new FakeTransport(welcomeFrame({ epoch: 1, tokenByte: 0x11 }));
    const second = new FakeTransport(welcomeFrame({ epoch: 2, tokenByte: 0x22 }));
    const { session, welcomes } = await fixture([first, second]);

    const sent = [1, 2, 3, 4].map((sequence) =>
      track(session.sendReliableCommand(sequence, new Uint8Array([sequence]))),
    );
    await until(() => first.lanes[0]?.sequences().length === 4);
    await idle();
    expect(sent.every((record) => !record.settled)).toBe(true);

    const lane = first.lanes[0];
    lane.ack(ackFrame(1, APPLIED));
    await until(() => sent[0].settled);
    expect(sent[0].value).toBe("applied");
    lane.ack(ackFrame(2, DUPLICATE));
    await until(() => sent[1].settled);
    expect(sent[1].value).toBe("duplicate");
    lane.ack(ackFrame(3, REJECTED));
    await until(() => sent[2].settled);
    expect(sent[2].error).toBeInstanceOf(Error);
    expect(String(sent[2].error.message)).toContain("rejected");
    expect(sent[3].settled).toBe(false);

    // Only the unacknowledged command is re-sent after a resume.
    first.drop();
    await until(() => welcomes() === 2);
    await until(() => second.lanes[0]?.sequences().length === 1);
    await idle();
    expect(second.lanes[0].frames().header).toEqual(HEADER);
    expect(second.lanes[0].sequences()).toEqual([4]);
    second.lanes[0].ack(ackFrame(4, APPLIED));
    await until(() => sent[3].settled);
    expect(sent[3].value).toBe("applied");
    session.close();
  });

  test("after a reconnect, unacked one-shots are re-sent in order before newer ones", async () => {
    const first = new FakeTransport(welcomeFrame({ epoch: 1, tokenByte: 0x11 }));
    const second = new FakeTransport(welcomeFrame({ epoch: 2, tokenByte: 0x22 }), {
      ready: false,
    });
    const { session, welcomes } = await fixture([first, second]);

    const sent = [10, 11, 12].map((sequence) =>
      track(session.sendReliableCommand(sequence, new Uint8Array([sequence]))),
    );
    await until(() => first.lanes[0]?.sequences().length === 3);
    first.lanes[0].ack(ackFrame(10, APPLIED));
    await until(() => sent[0].settled);

    first.drop();
    await idle();
    // Sent while the resume is still pending: queued behind the unacknowledged ones.
    const later = track(session.sendReliableCommand(13, new Uint8Array([13])));
    const held = session.sendCommand(14, new Uint8Array([14]));
    await idle();
    expect(second.lanes).toHaveLength(0);

    second.releaseReady();
    await until(() => welcomes() === 2);
    await held;
    await until(() => second.lanes[0]?.sequences().length === 3);
    await idle();
    expect(second.lanes).toHaveLength(1);
    expect(second.lanes[0].frames().header).toEqual(HEADER);
    expect(second.lanes[0].sequences()).toEqual([11, 12, 13]);
    expect(second.datagramSequences()).toEqual([14]);

    // Sequence 11 reached the authority before the outage; its ack did not.
    second.lanes[0].ack(ackFrame(11, DUPLICATE), ackFrame(12, APPLIED), ackFrame(13, APPLIED));
    await until(() => later.settled);
    expect([sent[1].value, sent[2].value, later.value]).toEqual([
      "duplicate",
      "applied",
      "applied",
    ]);
    session.close();
  });

  test("dropped datagrams cannot lose a one-shot", async () => {
    const first = new FakeTransport(welcomeFrame());
    const { session } = await fixture([first]);
    // Every datagram write resolves locally, and the network loses all of them.
    for (const sequence of [1, 2, 3]) await session.sendCommand(sequence, new Uint8Array([0]));
    const attack = track(session.sendReliableCommand(4, new Uint8Array([4])));
    for (const sequence of [5, 6]) await session.sendCommand(sequence, new Uint8Array([0]));
    first.datagramsSent.length = 0;

    await until(() => first.lanes[0]?.sequences().length === 1);
    expect(first.lanes[0].sequences()).toEqual([4]);
    first.lanes[0].ack(ackFrame(4, APPLIED));
    await until(() => attack.settled);
    expect(attack.value).toBe("applied");
    session.close();
  });

  test("acks split across chunks or batched into one chunk are decoded", async () => {
    const first = new FakeTransport(welcomeFrame());
    const { session, errors } = await fixture([first]);
    const sent = [1, 2, 3].map((sequence) =>
      track(session.sendReliableCommand(sequence, new Uint8Array([sequence]))),
    );
    await until(() => first.lanes[0]?.sequences().length === 3);
    const lane = first.lanes[0];
    const one = ackFrame(1, APPLIED);
    lane.ack(one.subarray(0, 2), one.subarray(2, 5));
    await idle();
    expect(sent[0].settled).toBe(false);
    const rest = new Uint8Array([
      ...one.subarray(5),
      ...ackFrame(2, APPLIED),
      ...ackFrame(3, APPLIED),
    ]);
    lane.ack(rest);
    await until(() => sent.every((record) => record.settled));
    expect(sent.map((record) => record.value)).toEqual(["applied", "applied", "applied"]);
    expect(errors).toEqual([]);
    expect(first.closedFlag).toBe(false);
    session.close();
  });

  for (const [name, respond] of [
    ["an unknown outcome", (lane) => lane.ack(ackFrame(1, 9))],
    ["a wrong frame kind", (lane) => lane.ack(ackFrame(1, APPLIED, { kind: 5 }))],
    ["a wrong protocol version", (lane) => lane.ack(ackFrame(1, APPLIED, { version: 2 }))],
    ["a zero sequence", (lane) => lane.ack(ackFrame(0, APPLIED))],
    ["an ack for a command not in flight first", (lane) => lane.ack(ackFrame(2, APPLIED))],
    [
      "an ack stream that ends mid-frame",
      (lane) => {
        lane.ack(ackFrame(1, APPLIED).subarray(0, 3));
        lane.finish();
      },
    ],
    ["an ack stream that ends with commands in flight", (lane) => lane.finish()],
  ]) {
    test(`${name} fails closed and the command is re-sent after the resume`, async () => {
      const first = new FakeTransport(welcomeFrame({ epoch: 1, tokenByte: 0x11 }));
      const second = new FakeTransport(welcomeFrame({ epoch: 2, tokenByte: 0x22 }));
      const { session, errors, welcomes } = await fixture([first, second]);
      const sent = [1, 2].map((sequence) =>
        track(session.sendReliableCommand(sequence, new Uint8Array([sequence]))),
      );
      await until(() => first.lanes[0]?.sequences().length === 2);

      respond(first.lanes[0]);
      await until(() => first.closedFlag);
      await until(() => welcomes() === 2);
      expect(errors.length).toBeGreaterThan(0);
      expect(sent.some((record) => record.settled)).toBe(false);

      await until(() => second.lanes[0]?.sequences().length === 2);
      expect(second.lanes[0].sequences()).toEqual([1, 2]);
      second.lanes[0].ack(ackFrame(1, APPLIED), ackFrame(2, APPLIED));
      await until(() => sent.every((record) => record.settled));
      expect(sent.map((record) => record.value)).toEqual(["applied", "applied"]);
      session.close();
    });
  }

  test("reliable sequences must be non-zero and strictly increase", async () => {
    const first = new FakeTransport(welcomeFrame());
    const { session } = await fixture([first]);
    await expect(session.sendReliableCommand(0, new Uint8Array([0]))).rejects.toThrow();
    const accepted = track(session.sendReliableCommand(5, new Uint8Array([5])));
    await expect(session.sendReliableCommand(5, new Uint8Array([5]))).rejects.toThrow();
    await expect(session.sendReliableCommand(4, new Uint8Array([4]))).rejects.toThrow();
    await until(() => first.lanes[0]?.sequences().length === 1);
    await idle();
    expect(first.lanes[0].sequences()).toEqual([5]);
    first.lanes[0].ack(ackFrame(5, APPLIED));
    await until(() => accepted.settled);
    session.close();
  });

  test("bounds unacknowledged one-shots", async () => {
    const first = new FakeTransport(welcomeFrame({ epoch: 1, tokenByte: 0x11 }));
    const blocked = new FakeTransport(welcomeFrame({ epoch: 2, tokenByte: 0x22 }), {
      ready: false,
    });
    const { session } = await fixture([first, blocked]);
    first.drop();
    await idle();
    const pending = [];
    for (let sequence = 1; sequence <= 256; sequence += 1) {
      pending.push(track(session.sendReliableCommand(sequence, new Uint8Array([1]))));
    }
    await expect(session.sendReliableCommand(257, new Uint8Array([1]))).rejects.toThrow("256");
    await idle();
    expect(pending.some((record) => record.settled)).toBe(false);
    session.close();
  });

  test("closing the session rejects unacknowledged one-shots", async () => {
    const first = new FakeTransport(welcomeFrame());
    const { session } = await fixture([first]);
    const attack = track(session.sendReliableCommand(1, new Uint8Array([1])));
    await until(() => first.lanes[0]?.sequences().length === 1);
    session.close();
    await until(() => attack.settled);
    expect(attack.error).toBeInstanceOf(Error);
    await expect(session.sendReliableCommand(2, new Uint8Array([2]))).rejects.toThrow();
  });

  test("an expired reconnect grace rejects unacknowledged one-shots", async () => {
    let now = 0;
    const first = new FakeTransport(welcomeFrame({ tickHz: 1000, graceTicks: 1n }));
    const disconnected = deferred();
    const session = new DedicatedGameSession({
      endpoint: "https://game.example/arpg",
      transportFactory: (url) => {
        if (!url.includes("/reconnect/")) return first;
        throw new Error("server still unavailable");
      },
      reconnectDelayMs: 10,
      now: () => now,
      sleep: async (milliseconds) => {
        now += milliseconds;
      },
    });
    await session.connect({
      onStateChange: (state) => {
        if (state === "disconnected") disconnected.resolve();
      },
    });
    const attack = track(session.sendReliableCommand(1, new Uint8Array([1])));
    await until(() => first.lanes[0]?.sequences().length === 1);
    first.drop();
    await disconnected.promise;
    await until(() => attack.settled);
    expect(attack.error).toBeInstanceOf(Error);
    session.close();
  });
});
