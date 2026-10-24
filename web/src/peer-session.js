import { t } from "./messages.ts";
import { contentRevisionMismatch, decodeSnapshot } from "./wire-protocol.js";

// ARPG admission and projection boundary. The foundation continues to own
// signaling, connection recovery, and WebRTC lifecycle.
export function attachPeerGameSession({
  session,
  role,
  isCurrent,
  getGame,
  onPlayer,
  onSnapshot,
  onStatus,
  localContentRevision,
  onIncompatible,
  onLinkRecovered = () => true,
  onHostLinkLost = () => {},
}) {
  const players = new Map();
  // Guest only: whether the host assigned this guest a player, and whether the host link has
  // left `connected` since. A link the foundation recovers in place (ICE restart) keeps the
  // same player and sends no new `peer-ready`, so the guest learns of it from the state.
  let guestAssigned = false;
  let hostLinkInterrupted = false;
  // Guests refuse a host whose content bundle differs from their own build.
  const incompatible = (snapshot) => {
    const reason = contentRevisionMismatch(localContentRevision(), snapshot, "host");
    if (reason) onIncompatible(reason);
    return reason !== null;
  };
  const detach = [];
  const listen = (name, handler) => {
    const listener = (event) => {
      if (!isCurrent()) return;
      try {
        handler(event.detail);
      } catch (error) {
        onStatus(t("status.peerRejected", { error: String(error) }));
      }
    };
    session.addEventListener(name, listener);
    detach.push(() => session.removeEventListener(name, listener));
  };

  listen("peer-ready", ({ peerId }) => {
    if (role !== "host") {
      onStatus(t("status.peerWaiting"));
      return;
    }
    let assigned = players.get(peerId);
    if (!assigned) {
      const used = new Set(players.values());
      assigned = [2, 3, 4].find((candidate) => !used.has(candidate));
      if (!assigned) {
        onStatus(t("status.lobbyFull"));
        return;
      }
      getGame().addPlayer(assigned);
      players.set(peerId, assigned);
    }
    session.sendReliable(peerId, {
      kind: "welcome",
      playerId: assigned,
      encodedSnapshot: getGame().snapshotJson(),
    });
    onStatus(t("status.peerJoined", { player: String(assigned) }));
  });

  listen("reliable", ({ peerId, data }) => {
    if (role === "host" && data?.kind === "command") {
      const assigned = players.get(peerId);
      if (!assigned) return;
      if (
        !Number.isInteger(data.sequence) ||
        data.sequence <= 0 ||
        data.sequence > 0xffff_ffff ||
        typeof data.encoded !== "string" ||
        data.encoded.length > 1024
      )
        throw new Error("Invalid peer command envelope");
      getGame().applyCommand(assigned, data.sequence, data.encoded);
    } else if (
      role === "guest" &&
      peerId === session.hostParticipantId &&
      data?.kind === "welcome"
    ) {
      const snapshot = decodeSnapshot(data.encodedSnapshot);
      if (incompatible(snapshot)) return;
      if (
        !Number.isInteger(data.playerId) ||
        data.playerId < 2 ||
        data.playerId > 4 ||
        !snapshot.players.some((player) => player.id === data.playerId)
      )
        throw new Error("Invalid host player assignment");
      guestAssigned = true;
      hostLinkInterrupted = false;
      onPlayer(data.playerId);
      onSnapshot(snapshot);
      onStatus(t("status.connectedAs", { player: String(data.playerId) }));
    }
  });

  listen("realtime", ({ peerId, data }) => {
    if (role !== "guest" || peerId !== session.hostParticipantId || data?.kind !== "snapshot")
      return;
    const snapshot = decodeSnapshot(data.encoded);
    if (incompatible(snapshot)) return;
    onSnapshot(snapshot);
  });

  listen("participant-disconnected", ({ participantId }) => {
    if (role !== "host") return;
    const assigned = players.get(participantId);
    if (!assigned) return;
    players.delete(participantId);
    getGame()?.removePlayer(assigned);
    onStatus(t("status.peerLeft", { player: String(assigned) }));
  });

  listen("peer-statechange", ({ peerId, state }) => {
    if (role !== "guest" || !guestAssigned || peerId !== session.hostParticipantId) return;
    if (state !== "connected") {
      hostLinkInterrupted = true;
      return;
    }
    retryLinkRecovery();
  });

  // Guest only: the foundation gave up recovering the link to the host. This is the
  // authoritative loss (#176): the guest stops acting until it joins again.
  listen("peer-recovery-exhausted", ({ peerId }) => {
    if (role !== "guest" || !guestAssigned || peerId !== session.hostParticipantId) return;
    guestAssigned = false;
    hostLinkInterrupted = false;
    onHostLinkLost();
  });

  // The recovery stays pending until the re-assertion actually reaches the host link, so a
  // `connected` that precedes a sendable reliable channel retries once the channel opens.
  const retryLinkRecovery = () => {
    if (!hostLinkInterrupted) return;
    if (onLinkRecovered() !== false) hostLinkInterrupted = false;
  };

  listen("channel-open", ({ peerId, kind }) => {
    if (role !== "guest" || !guestAssigned || peerId !== session.hostParticipantId) return;
    if (kind === "reliable") retryLinkRecovery();
  });

  listen("statechange", (detail) => {
    if (detail?.state) onStatus(t("status.network", { state: String(detail.state) }));
  });

  return () => {
    for (const remove of detach) remove();
    players.clear();
  };
}
