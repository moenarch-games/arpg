const GAME_SERVER_PROTOCOL_VERSION = 3;
const COMMAND_KIND = 1;
const SNAPSHOT_KIND = 2;
const WELCOME_KIND = 3;
const SNAPSHOT_FRAGMENT_KIND = 4;
const RELIABLE_COMMAND_ACK_KIND = 6;
const SNAPSHOT_FRAGMENT_HEADER_BYTES = 14;
const MAX_SNAPSHOT_FRAGMENTS = 64;
const COMMAND_HEADER_BYTES = 8;
const SNAPSHOT_HEADER_BYTES = 20;
const WELCOME_BYTES = 46;
const RECONNECT_TOKEN_BYTES = 16;
const MAX_COMMAND_PAYLOAD_BYTES = 1024;
const MAX_SNAPSHOT_PAYLOAD_BYTES = 0xffff;
const MAX_SNAPSHOT_FRAME_BYTES = SNAPSHOT_HEADER_BYTES + MAX_SNAPSHOT_PAYLOAD_BYTES;
// Reassembly bounds mirror game-server's SnapshotReassembler.
const SNAPSHOT_REASSEMBLY_MAX_PENDING = 4;
const SNAPSHOT_REASSEMBLY_MAX_BUFFERED_BYTES = 2 * MAX_SNAPSHOT_FRAME_BYTES;
const SNAPSHOT_REASSEMBLY_MAX_IDLE_DATAGRAMS =
  SNAPSHOT_REASSEMBLY_MAX_PENDING * MAX_SNAPSHOT_FRAGMENTS;
const MAX_PENDING_COMMANDS = 256;
// game-server ADR 0008: the reliable command lane is one bidirectional stream per connection
// that starts with `GSCL`, the lane format version and the game protocol version.
const COMMAND_LANE_MAGIC = [0x47, 0x53, 0x43, 0x4c];
const COMMAND_LANE_FORMAT_VERSION = 1;
const RELIABLE_COMMAND_ACK_BYTES = 7;
const RELIABLE_COMMAND_OUTCOMES = [null, "applied", "duplicate", "rejected"];
const MAX_UNACKED_RELIABLE_COMMANDS = 256;
const DEFAULT_RECONNECT_DELAY_MS = 250;
const DEFAULT_CONNECT_TIMEOUT_MS = 10_000;
const MAX_RECONNECT_DELAY_MS = 1000;
const FNV_OFFSET_BASIS = 0xcbf29ce484222325n;
const FNV_PRIME = 0x100000001b3n;
const U64_MASK = 0xffffffffffffffffn;

function asBytes(value) {
  if (value instanceof Uint8Array) return value;
  if (value instanceof ArrayBuffer) return new Uint8Array(value);
  if (ArrayBuffer.isView(value)) {
    return new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
  }
  throw new TypeError("Expected binary WebTransport data");
}

function requireHeader(bytes, kind) {
  if (bytes[0] !== GAME_SERVER_PROTOCOL_VERSION) {
    throw new Error(`Unsupported game-server protocol version ${bytes[0]}`);
  }
  if (bytes[1] !== kind) {
    throw new Error(`Unexpected game-server frame kind ${bytes[1]}`);
  }
}

function fnvUpdate(hash, bytes) {
  let next = hash;
  for (const byte of bytes) {
    next ^= BigInt(byte);
    next = (next * FNV_PRIME) & U64_MASK;
  }
  return next;
}

function u64Bytes(value) {
  const bytes = new Uint8Array(8);
  new DataView(bytes.buffer).setBigUint64(0, BigInt(value), false);
  return bytes;
}

function delay(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

export function snapshotHash(tick, payload) {
  const bytes = asBytes(payload);
  let hash = fnvUpdate(FNV_OFFSET_BASIS, u64Bytes(tick));
  hash = fnvUpdate(hash, u64Bytes(bytes.byteLength));
  return fnvUpdate(hash, bytes);
}

export function encodeGameServerCommand(sequence, payload) {
  if (!Number.isInteger(sequence) || sequence <= 0 || sequence > 0xffff_ffff) {
    throw new Error("Command sequence must be a non-zero u32");
  }
  const bytes = asBytes(payload);
  if (bytes.byteLength > MAX_COMMAND_PAYLOAD_BYTES) {
    throw new Error(`Command payload exceeds ${MAX_COMMAND_PAYLOAD_BYTES} bytes`);
  }
  const frame = new Uint8Array(COMMAND_HEADER_BYTES + bytes.byteLength);
  const view = new DataView(frame.buffer);
  frame[0] = GAME_SERVER_PROTOCOL_VERSION;
  frame[1] = COMMAND_KIND;
  view.setUint32(2, sequence, false);
  view.setUint16(6, bytes.byteLength, false);
  frame.set(bytes, COMMAND_HEADER_BYTES);
  return frame;
}

export function encodeCommandLaneHeader() {
  return new Uint8Array([
    ...COMMAND_LANE_MAGIC,
    COMMAND_LANE_FORMAT_VERSION,
    GAME_SERVER_PROTOCOL_VERSION,
  ]);
}

export function decodeReliableCommandAck(frame) {
  const bytes = asBytes(frame);
  if (bytes.byteLength !== RELIABLE_COMMAND_ACK_BYTES) {
    throw new Error(
      `Reliable command ack must contain exactly ${RELIABLE_COMMAND_ACK_BYTES} bytes`,
    );
  }
  requireHeader(bytes, RELIABLE_COMMAND_ACK_KIND);
  const sequence = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength).getUint32(
    2,
    false,
  );
  if (sequence === 0) throw new Error("Reliable command ack names sequence zero");
  const outcome = RELIABLE_COMMAND_OUTCOMES[bytes[6]];
  if (!outcome) throw new Error(`Unknown reliable command ack outcome ${bytes[6]}`);
  return { sequence, outcome };
}

export function decodeGameServerSnapshot(frame) {
  const bytes = asBytes(frame);
  if (bytes.byteLength < SNAPSHOT_HEADER_BYTES) {
    throw new Error(`Snapshot frame is shorter than ${SNAPSHOT_HEADER_BYTES} bytes`);
  }
  requireHeader(bytes, SNAPSHOT_KIND);
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const tick = view.getBigUint64(2, false);
  const stateHash = view.getBigUint64(10, false);
  const payloadLength = view.getUint16(18, false);
  if (payloadLength > MAX_SNAPSHOT_PAYLOAD_BYTES) {
    throw new Error("Snapshot payload exceeds protocol limit");
  }
  if (bytes.byteLength !== SNAPSHOT_HEADER_BYTES + payloadLength) {
    throw new Error("Snapshot frame length does not match its declared payload length");
  }
  const payload = bytes.slice(SNAPSHOT_HEADER_BYTES);
  const expectedHash = snapshotHash(tick, payload);
  if (expectedHash !== stateHash) {
    throw new Error("Snapshot state hash does not match payload");
  }
  return { tick, stateHash, payload };
}

export function decodeGameServerSnapshotFragment(datagram) {
  const bytes = asBytes(datagram);
  if (bytes.byteLength < SNAPSHOT_FRAGMENT_HEADER_BYTES) {
    throw new Error(`Snapshot fragment is shorter than ${SNAPSHOT_FRAGMENT_HEADER_BYTES} bytes`);
  }
  requireHeader(bytes, SNAPSHOT_FRAGMENT_KIND);
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const tick = view.getBigUint64(2, false);
  const index = bytes[10];
  const count = bytes[11];
  if (count === 0 || count > MAX_SNAPSHOT_FRAGMENTS) {
    throw new Error(`Snapshot fragment count ${count} is outside 1..=${MAX_SNAPSHOT_FRAGMENTS}`);
  }
  if (index >= count) {
    throw new Error(`Snapshot fragment index ${index} is outside fragment count ${count}`);
  }
  const chunkLength = view.getUint16(12, false);
  if (chunkLength === 0) throw new Error("Snapshot fragment carries no bytes");
  if (bytes.byteLength !== SNAPSHOT_FRAGMENT_HEADER_BYTES + chunkLength) {
    throw new Error("Snapshot fragment length does not match its declared chunk length");
  }
  return { tick, index, count, chunk: bytes.slice(SNAPSHOT_FRAGMENT_HEADER_BYTES) };
}

/**
 * Bounded reassembly of game-server snapshot datagrams, which arrive either
 * whole or as fragments when a frame exceeds the path's datagram budget.
 * Delivered ticks strictly increase; stale, duplicate and evicted data is
 * ignored, and malformed or inconsistent data throws.
 */
export class SnapshotReassembler {
  constructor() {
    this.pending = new Map();
    this.bufferedBytes = 0;
    this.newestTick = null;
    this.datagrams = 0;
  }

  accept(datagram) {
    this.datagrams += 1;
    this.expireIdle();
    const bytes = asBytes(datagram);
    if (bytes[1] === SNAPSHOT_FRAGMENT_KIND) {
      return this.acceptFragment(decodeGameServerSnapshotFragment(bytes));
    }
    const snapshot = decodeGameServerSnapshot(bytes);
    return this.isStale(snapshot.tick) ? null : this.deliver(snapshot);
  }

  acceptFragment({ tick, index, count, chunk }) {
    if (this.isStale(tick)) return null;
    const existing = this.pending.get(tick);
    if (existing) {
      if (existing.chunks.length !== count) {
        throw new Error(`Snapshot fragments for tick ${tick} are inconsistent`);
      }
      if (existing.chunks[index]) return null;
      if (existing.bytes + chunk.byteLength > MAX_SNAPSHOT_FRAME_BYTES) {
        this.removePending(tick);
        throw new Error("Reassembled snapshot exceeds the protocol frame limit");
      }
    } else if (
      this.pending.size >= SNAPSHOT_REASSEMBLY_MAX_PENDING &&
      !this.evictOldestBefore(tick)
    ) {
      return null;
    }
    while (this.bufferedBytes + chunk.byteLength > SNAPSHOT_REASSEMBLY_MAX_BUFFERED_BYTES) {
      if (!this.evictOldestBefore(tick)) {
        this.removePending(tick);
        return null;
      }
    }

    let pending = this.pending.get(tick);
    if (!pending) {
      pending = { chunks: new Array(count).fill(null), received: 0, bytes: 0, lastStoredAt: 0 };
      this.pending.set(tick, pending);
    }
    pending.chunks[index] = chunk;
    pending.received += 1;
    pending.bytes += chunk.byteLength;
    pending.lastStoredAt = this.datagrams;
    this.bufferedBytes += chunk.byteLength;
    if (pending.received < count) return null;

    this.removePending(tick);
    const frame = new Uint8Array(pending.bytes);
    let offset = 0;
    for (const part of pending.chunks) {
      frame.set(part, offset);
      offset += part.byteLength;
    }
    const snapshot = decodeGameServerSnapshot(frame);
    if (snapshot.tick !== tick) {
      throw new Error(`Snapshot fragments for tick ${tick} are inconsistent`);
    }
    return this.deliver(snapshot);
  }

  isStale(tick) {
    return this.newestTick !== null && tick <= this.newestTick;
  }

  deliver(snapshot) {
    for (const tick of [...this.pending.keys()]) {
      if (tick <= snapshot.tick) this.removePending(tick);
    }
    this.newestTick = snapshot.tick;
    return snapshot;
  }

  removePending(tick) {
    const pending = this.pending.get(tick);
    if (!pending) return null;
    this.pending.delete(tick);
    this.bufferedBytes -= pending.bytes;
    return pending;
  }

  evictOldestBefore(tick) {
    let oldest = null;
    for (const candidate of this.pending.keys()) {
      if (candidate < tick && (oldest === null || candidate < oldest)) oldest = candidate;
    }
    if (oldest === null) return false;
    this.removePending(oldest);
    return true;
  }

  expireIdle() {
    for (const [tick, pending] of [...this.pending]) {
      if (this.datagrams - pending.lastStoredAt > SNAPSHOT_REASSEMBLY_MAX_IDLE_DATAGRAMS) {
        this.removePending(tick);
      }
    }
  }
}

export function decodeGameServerWelcome(frame) {
  const bytes = asBytes(frame);
  if (bytes.byteLength !== WELCOME_BYTES) {
    throw new Error(`Welcome frame must contain exactly ${WELCOME_BYTES} bytes`);
  }
  requireHeader(bytes, WELCOME_KIND);
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  return {
    playerId: view.getUint32(2, false),
    tickHz: view.getUint16(6, false),
    maxPlayers: view.getUint16(8, false),
    currentTick: view.getBigUint64(10, false),
    connectionEpoch: view.getUint32(18, false),
    reconnectToken: bytes.slice(22, 22 + RECONNECT_TOKEN_BYTES),
    reconnectGraceTicks: view.getBigUint64(38, false),
  };
}

export function reconnectTokenHex(token) {
  const bytes = asBytes(token);
  if (bytes.byteLength !== RECONNECT_TOKEN_BYTES) {
    throw new Error(`Reconnect token must contain exactly ${RECONNECT_TOKEN_BYTES} bytes`);
  }
  return Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
}

export function reconnectEndpoint(endpoint, token) {
  const url = new URL(endpoint);
  const basePath = url.pathname.replace(/\/+$/, "");
  url.pathname = `${basePath}/reconnect/${reconnectTokenHex(token)}`;
  url.search = "";
  url.hash = "";
  return url.toString();
}

export function reconnectGraceMilliseconds(welcome) {
  if (!welcome || !Number.isInteger(welcome.tickHz) || welcome.tickHz <= 0) return 0;
  const ticks = BigInt(welcome.reconnectGraceTicks);
  const tickHz = BigInt(welcome.tickHz);
  const milliseconds = (ticks * 1000n + tickHz - 1n) / tickHz;
  const maximum = BigInt(Number.MAX_SAFE_INTEGER);
  return Number(milliseconds > maximum ? maximum : milliseconds);
}

async function boundedWait(promise, signal) {
  signal.throwIfAborted();
  let rejectOnAbort;
  const aborted = new Promise((_resolve, reject) => {
    rejectOnAbort = () => reject(signal.reason);
    signal.addEventListener("abort", rejectOnAbort, { once: true });
  });
  try {
    return await Promise.race([promise, aborted]);
  } finally {
    signal.removeEventListener("abort", rejectOnAbort);
  }
}

async function readAll(stream, signal) {
  const reader = stream.getReader();
  const chunks = [];
  let total = 0;
  try {
    while (true) {
      const { value, done } = await boundedWait(reader.read(), signal);
      if (done) break;
      const bytes = asBytes(value);
      total += bytes.byteLength;
      if (total > WELCOME_BYTES) throw new Error(`Welcome stream exceeds ${WELCOME_BYTES} bytes`);
      chunks.push(bytes);
    }
  } finally {
    reader.releaseLock();
  }
  const result = new Uint8Array(total);
  let offset = 0;
  for (const chunk of chunks) {
    result.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return result;
}

export class DedicatedGameSession {
  constructor({
    endpoint,
    transportFactory = (url) => new WebTransport(url),
    reconnectDelayMs = DEFAULT_RECONNECT_DELAY_MS,
    connectTimeoutMs = DEFAULT_CONNECT_TIMEOUT_MS,
    sleep = delay,
    now = () => Date.now(),
  }) {
    this.endpoint = endpoint;
    this.transportFactory = transportFactory;
    this.reconnectDelayMs = Math.max(1, reconnectDelayMs);
    if (!Number.isFinite(connectTimeoutMs) || connectTimeoutMs <= 0)
      throw new Error("Connection timeout must be positive and finite");
    this.connectTimeoutMs = connectTimeoutMs;
    this.connectionAbort = null;
    this.sleep = sleep;
    this.now = now;
    this.transport = null;
    this.datagramWriter = null;
    this.closedByClient = false;
    this.callbacks = {};
    this.welcome = null;
    this.connectionGeneration = 0;
    this.reconnectRequested = false;
    this.reconnectTask = null;
    this.pendingCommands = [];
    // Reliable one-shots in send order until the authority acknowledges them; they survive
    // reconnects and are re-sent on the next connection's lane.
    this.reliableCommands = [];
    this.lastReliableSequence = 0;
    // The current connection's command lane, opened on demand; null between connections.
    this.lane = null;
    // The generation whose handshake completed, so the lane opens only on a live connection.
    this.establishedGeneration = null;
  }

  async connect(callbacks = {}) {
    if (this.transport || this.reconnectTask) {
      throw new Error("Dedicated session is already connected");
    }
    this.closedByClient = false;
    this.callbacks = callbacks;
    this.welcome = null;
    this.pendingCommands = [];
    this.reliableCommands = [];
    this.lastReliableSequence = 0;
    this.callbacks.onStateChange?.("connecting");
    try {
      return await this.openTransport(this.endpoint, false);
    } catch (error) {
      this.close();
      throw error;
    }
  }

  isCurrentTransport(transport, generation) {
    return (
      !this.closedByClient &&
      this.transport === transport &&
      this.connectionGeneration === generation
    );
  }

  requireCurrentTransport(transport, generation) {
    if (!this.isCurrentTransport(transport, generation)) {
      throw new Error("Dedicated connection was superseded while opening");
    }
  }

  validateResumedWelcome(previous, welcome) {
    if (!previous) throw new Error("Cannot resume without prior reconnect identity");
    if (welcome.playerId !== previous.playerId) {
      throw new Error("Dedicated reconnect changed authoritative player identity");
    }
    if (welcome.connectionEpoch <= previous.connectionEpoch) {
      throw new Error("Dedicated reconnect did not advance the connection epoch");
    }
  }

  async openTransport(endpoint, resumed, timeoutMs = this.connectTimeoutMs) {
    const generation = ++this.connectionGeneration;
    const abort = new AbortController();
    this.connectionAbort = abort;
    const timeout = setTimeout(
      () => abort.abort(new Error("Dedicated handshake timed out")),
      timeoutMs,
    );
    let transport = null;
    try {
      transport = this.transportFactory(endpoint);
      this.transport = transport;
      await boundedWait(transport.ready, abort.signal);
      this.requireCurrentTransport(transport, generation);

      const streams = transport.incomingUnidirectionalStreams.getReader();
      let welcomeStream;
      let done;
      try {
        ({ value: welcomeStream, done } = await boundedWait(streams.read(), abort.signal));
      } finally {
        streams.releaseLock();
      }
      if (done || !welcomeStream) throw new Error("Dedicated server closed before welcome");

      const welcome = decodeGameServerWelcome(await readAll(welcomeStream, abort.signal));
      this.requireCurrentTransport(transport, generation);
      const previous = this.welcome;
      if (resumed) this.validateResumedWelcome(previous, welcome);

      this.datagramWriter = transport.datagrams.writable.getWriter();
      this.welcome = welcome;
      await boundedWait(this.flushPendingCommands(transport, generation), abort.signal);
      this.requireCurrentTransport(transport, generation);

      this.establishedGeneration = generation;
      this.callbacks.onWelcome?.(welcome, { resumed });
      this.callbacks.onStateChange?.("connected");
      void this.consumeSnapshots(transport, generation);
      transport.closed
        .then(() => this.handleTransportFailure(transport, generation))
        .catch((error) => this.handleTransportFailure(transport, generation, error));
      // Unacknowledged one-shots from the previous connection go out first, in order.
      this.ensureLane();
      return welcome;
    } catch (error) {
      if (transport) this.detachTransport(transport, generation, true);
      throw error;
    } finally {
      clearTimeout(timeout);
    }
  }

  async consumeSnapshots(transport, generation) {
    let reader;
    try {
      reader = transport.datagrams.readable.getReader();
      const reassembler = new SnapshotReassembler();
      while (this.isCurrentTransport(transport, generation)) {
        const { value, done } = await reader.read();
        if (!this.isCurrentTransport(transport, generation)) return;
        if (done) return;
        const snapshot = reassembler.accept(value);
        if (snapshot) this.callbacks.onSnapshot?.(snapshot);
      }
    } catch (error) {
      this.handleTransportFailure(transport, generation, error);
    } finally {
      reader?.releaseLock();
    }
  }

  detachTransport(transport, generation, closeTransport) {
    if (this.transport !== transport || this.connectionGeneration !== generation) return false;
    this.connectionGeneration += 1;
    this.connectionAbort?.abort(new Error("Dedicated connection was superseded"));
    this.connectionAbort = null;
    try {
      this.datagramWriter?.releaseLock();
    } catch {
      // A pending write owns the writer until it settles.
    }
    this.datagramWriter = null;
    this.transport = null;
    this.lane = null;
    if (closeTransport) {
      try {
        transport.close({ closeCode: 0, reason: "connection replaced" });
      } catch {
        // A failed transport may already be closed.
      }
    }
    return true;
  }

  handleTransportFailure(transport, generation, error = null) {
    if (this.closedByClient || !this.isCurrentTransport(transport, generation)) return;
    if (!this.detachTransport(transport, generation, true)) return;
    if (error) this.callbacks.onError?.(error);
    if (!this.welcome) {
      this.callbacks.onStateChange?.("disconnected");
      return;
    }
    this.requestReconnect();
  }

  requestReconnect() {
    if (this.closedByClient || !this.welcome) return;
    this.reconnectRequested = true;
    if (this.reconnectTask) return;
    this.reconnectTask = this.runReconnectLoop().finally(() => {
      this.reconnectTask = null;
      if (this.reconnectRequested && !this.closedByClient) this.requestReconnect();
    });
  }

  async runReconnectLoop() {
    if (!this.reconnectRequested || this.closedByClient || !this.welcome) return;
    this.reconnectRequested = false;
    let deadline = this.now() + reconnectGraceMilliseconds(this.welcome);
    let retryDelay = 0;
    let lastError = null;
    this.callbacks.onStateChange?.("connecting");

    while (!this.closedByClient && this.welcome && this.now() <= deadline) {
      if (retryDelay > 0) {
        const remaining = Math.max(0, deadline - this.now());
        if (remaining === 0) break;
        await this.sleep(Math.min(retryDelay, remaining));
        if (this.closedByClient) return;
      }

      const token = this.welcome.reconnectToken;
      const attemptedToken = reconnectTokenHex(token);
      try {
        await this.openTransport(
          reconnectEndpoint(this.endpoint, token),
          true,
          Math.max(1, Math.min(this.connectTimeoutMs, deadline - this.now())),
        );
        return;
      } catch (error) {
        lastError = error;
        if (this.welcome && reconnectTokenHex(this.welcome.reconnectToken) !== attemptedToken) {
          deadline = this.now() + reconnectGraceMilliseconds(this.welcome);
        }
        retryDelay =
          retryDelay === 0
            ? this.reconnectDelayMs
            : Math.min(retryDelay * 2, MAX_RECONNECT_DELAY_MS);
      }
    }

    if (this.closedByClient) return;
    this.pendingCommands = [];
    this.rejectReliableCommands(
      new Error("Dedicated reconnect grace expired before the command was acknowledged"),
    );
    this.welcome = null;
    if (lastError) this.callbacks.onError?.(lastError);
    this.callbacks.onStateChange?.("disconnected");
  }

  enqueueCommand(frame) {
    if (this.pendingCommands.length >= MAX_PENDING_COMMANDS) {
      throw new Error(`Dedicated reconnect command buffer exceeds ${MAX_PENDING_COMMANDS} entries`);
    }
    this.pendingCommands.push(frame.slice());
  }

  async flushPendingCommands(transport, generation) {
    while (this.pendingCommands.length > 0) {
      this.requireCurrentTransport(transport, generation);
      if (!this.datagramWriter) throw new Error("Dedicated datagram writer is unavailable");
      await this.datagramWriter.write(this.pendingCommands[0]);
      this.requireCurrentTransport(transport, generation);
      this.pendingCommands.shift();
    }
  }

  async sendCommand(sequence, payload) {
    const frame = encodeGameServerCommand(sequence, payload);
    if (this.closedByClient || !this.welcome) {
      throw new Error("Dedicated session is not connected");
    }
    if (!this.datagramWriter || !this.transport) {
      this.enqueueCommand(frame);
      return;
    }

    const transport = this.transport;
    const generation = this.connectionGeneration;
    try {
      await boundedWait(
        this.datagramWriter.write(frame),
        AbortSignal.any([this.connectionAbort.signal, AbortSignal.timeout(this.connectTimeoutMs)]),
      );
    } catch (error) {
      if (this.isCurrentTransport(transport, generation)) {
        this.enqueueCommand(frame);
        this.handleTransportFailure(transport, generation, error);
        return;
      }
      throw error;
    }
  }

  /**
   * Sends a one-shot command on the reliable command lane. Resolves with "applied" or
   * "duplicate" once the authority acknowledged it and rejects for a rejected command, or
   * when the session closes or its reconnect grace expires first. Until then the command is
   * buffered and re-sent, in order, on every new connection.
   */
  async sendReliableCommand(sequence, payload) {
    const frame = encodeGameServerCommand(sequence, payload);
    if (this.closedByClient || !this.welcome) {
      throw new Error("Dedicated session is not connected");
    }
    if (sequence <= this.lastReliableSequence) {
      throw new Error(
        `Reliable command sequence ${sequence} does not follow ${this.lastReliableSequence}`,
      );
    }
    if (this.reliableCommands.length >= MAX_UNACKED_RELIABLE_COMMANDS) {
      throw new Error(
        `Dedicated reliable command buffer exceeds ${MAX_UNACKED_RELIABLE_COMMANDS} unacknowledged entries`,
      );
    }
    this.lastReliableSequence = sequence;
    const acknowledged = new Promise((resolve, reject) => {
      this.reliableCommands.push({ sequence, frame: frame.slice(), resolve, reject });
    });
    this.ensureLane();
    if (this.lane?.writer) void this.pumpLane(this.lane);
    return acknowledged;
  }

  // Opens the connection's one command lane when one-shots wait and the connection is live.
  ensureLane() {
    if (this.lane || this.reliableCommands.length === 0 || this.closedByClient) return;
    const transport = this.transport;
    const generation = this.connectionGeneration;
    if (!transport || !this.connectionAbort || this.establishedGeneration !== generation) return;
    const lane = { transport, generation, writer: null, sent: 0, pumping: false };
    this.lane = lane;
    void this.openLane(lane, this.connectionAbort.signal);
  }

  async openLane(lane, signal) {
    try {
      const stream = await boundedWait(lane.transport.createBidirectionalStream(), signal);
      if (this.lane !== lane) return;
      const writer = stream.writable.getWriter();
      void this.readAcks(lane, stream.readable);
      await writer.write(encodeCommandLaneHeader());
      if (this.lane !== lane) return;
      lane.writer = writer;
      await this.pumpLane(lane);
    } catch (error) {
      this.failLane(lane, error);
    }
  }

  // Writes every buffered command the lane has not carried yet, in order.
  async pumpLane(lane) {
    if (lane.pumping || !lane.writer) return;
    lane.pumping = true;
    try {
      while (this.lane === lane && lane.sent < this.reliableCommands.length) {
        const { frame } = this.reliableCommands[lane.sent];
        lane.sent += 1;
        await lane.writer.write(frame);
      }
    } catch (error) {
      this.failLane(lane, error);
    } finally {
      lane.pumping = false;
    }
  }

  async readAcks(lane, readable) {
    const reader = readable.getReader();
    let buffered = new Uint8Array(0);
    try {
      while (true) {
        const { value, done } = await reader.read();
        if (this.lane !== lane) return;
        // The server ends its ack stream only after the client finished the lane, which
        // this client never does, so any end is a lane fault.
        if (done) {
          throw new Error(
            buffered.byteLength > 0
              ? "Reliable command lane ended inside an ack"
              : "Reliable command lane ended",
          );
        }
        const bytes = asBytes(value);
        const next = new Uint8Array(buffered.byteLength + bytes.byteLength);
        next.set(buffered);
        next.set(bytes, buffered.byteLength);
        buffered = next;
        while (buffered.byteLength >= RELIABLE_COMMAND_ACK_BYTES) {
          const ack = decodeReliableCommandAck(buffered.subarray(0, RELIABLE_COMMAND_ACK_BYTES));
          buffered = buffered.slice(RELIABLE_COMMAND_ACK_BYTES);
          this.acceptAck(lane, ack);
        }
      }
    } catch (error) {
      this.failLane(lane, error);
    } finally {
      reader.releaseLock();
    }
  }

  // Acks arrive in send order, so each must name the oldest command in flight on this lane.
  acceptAck(lane, { sequence, outcome }) {
    const oldest = this.reliableCommands[0];
    if (lane.sent === 0 || !oldest || oldest.sequence !== sequence) {
      throw new Error(`Unexpected reliable command ack for sequence ${sequence}`);
    }
    this.reliableCommands.shift();
    lane.sent -= 1;
    if (outcome === "rejected") {
      oldest.reject(new Error(`Dedicated command ${sequence} was rejected by the authority`));
    } else {
      oldest.resolve(outcome);
    }
  }

  // A lane fault closes the whole connection; unacknowledged commands stay buffered and are
  // re-sent after the resume, where the reliable watermark turns any already applied one
  // into a duplicate.
  failLane(lane, error) {
    if (this.lane !== lane) return;
    this.handleTransportFailure(lane.transport, lane.generation, error);
  }

  rejectReliableCommands(error) {
    const commands = this.reliableCommands;
    this.reliableCommands = [];
    this.lane = null;
    for (const command of commands) command.reject(error);
  }

  close() {
    this.closedByClient = true;
    this.reconnectRequested = false;
    this.connectionGeneration += 1;
    this.connectionAbort?.abort(new Error("Dedicated session was closed"));
    this.connectionAbort = null;
    try {
      this.datagramWriter?.releaseLock();
    } catch {
      // A pending write owns the writer until it settles.
    }
    this.datagramWriter = null;
    try {
      this.transport?.close({ closeCode: 0, reason: "client closed" });
    } catch {
      // The transport may already be closed.
    }
    this.transport = null;
    this.welcome = null;
    this.pendingCommands = [];
    this.rejectReliableCommands(new Error("Dedicated session was closed"));
  }
}
