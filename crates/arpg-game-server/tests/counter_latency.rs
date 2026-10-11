//! Counter window under remote latency (#131): peer-hosted and dedicated evidence.
//!
//! A successful block grants a counter opportunity whose eligibility is the half-open
//! interval `[usable_from_tick, expires_at_tick)`, compared with the tick at which the
//! authority *applies* a command. Neither topology compensates for latency: a command
//! counts at the tick it is applied, and carries no tick of its own. These tests drive both
//! authorities through their public seams with the real ARPG wire encoding:
//!
//! - peer-hosted: the host browser's `ArpgGame` (as `arpg-web-wasm` drives it), applying a
//!   guest command when it arrives on the reliable ordered channel;
//! - dedicated: `game_server::MatchRuntime` over [`GameServerAdapter`], admitting sessions
//!   and receiving commands as realtime datagrams (which may be reordered or duplicated).
//!
//! The remote player sees the snapshot that carries the opportunity one-way delay `d`
//! after the authority published it, reacts `r` ticks later, and its command reaches the
//! authority `d` later again: it is applied at `usable_from_tick + 2d + r`. Snapshots are
//! published every tick in both topologies, so there is no further cadence quantization.

use arpg_core::{
    ActionKind, ActionPhase, ArpgCommand, ArpgGame, ArpgSnapshot, AuthoritativeGame,
    CounterOpportunity, PlayerCommand, PlayerId, PlayerSnapshot, ScenarioId, StrikeResult,
    StrikeSource, StrikeTarget, TICK_HZ, TuningParameter, WorkbenchOperation, base_bundle,
};
use arpg_game_server::GameServerAdapter;
use arpg_protocol::{JsonProtocol, WireProtocol};
use game_server::{
    CommandOutcome, MatchRuntime, RECONNECT_TOKEN_BYTES, ReconnectToken, RuntimeError, SessionLease,
};

/// The remote player under test: the peer guest, or the second dedicated client.
const REMOTE: PlayerId = 2;
const SEED: u32 = 0xA420_0916;
const RECONNECT_GRACE_TICKS: u64 = 120;
/// Representative one-way delays.
const ONE_WAY_DELAYS_MS: [u64; 5] = [0, 25, 50, 75, 100];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Delivery {
    Applied,
    Stale,
}

/// One authority as the remote player's commands and snapshots reach it.
trait Authority {
    fn tick(&self) -> u64;
    /// Delivers one encoded command of the remote player now, at the current tick.
    fn deliver(&mut self, sequence: u32, command: ArpgCommand) -> Delivery;
    /// Runs one tick and returns the snapshot it publishes, decoded as a client does.
    fn advance(&mut self) -> ArpgSnapshot;
    /// The current authoritative snapshot, decoded as a client does.
    fn view(&self) -> ArpgSnapshot;
}

/// The scenario arena with the target in front of the remote player: the enemy scenario
/// with its brute moved beside the second spawn point, so it engages the remote player.
fn arena() -> ArpgGame {
    let mut game = ArpgGame::new_scenario(ScenarioId::Enemy, SEED).unwrap();
    let room = game.workbench_room().unwrap();
    let generated = game
        .snapshot()
        .unwrap()
        .monsters
        .iter()
        .find(|monster| monster.room_id == room)
        .unwrap()
        .id;
    game.apply_workbench(&WorkbenchOperation::RemoveMonster {
        monster_id: generated,
    })
    .unwrap();
    // The second player spawns 90 units off the centre along +z, facing +x.
    game.apply_workbench(&WorkbenchOperation::SpawnMonster {
        definition: "monster.brute".into(),
        offset: [150, 90],
    })
    .unwrap();
    game
}

/// Peer host: the host browser runs the authority and applies a guest command as soon as
/// the reliable channel delivers it (`peer-session.js` → `applyCommand`).
struct PeerHost {
    game: ArpgGame,
}

impl PeerHost {
    fn new() -> Self {
        let mut game = arena();
        game.add_player(1).unwrap(); // the host's own player
        game.add_player(REMOTE).unwrap(); // the guest
        Self { game }
    }
}

impl Authority for PeerHost {
    fn tick(&self) -> u64 {
        self.game.current_tick()
    }

    fn deliver(&mut self, sequence: u32, command: ArpgCommand) -> Delivery {
        // What `arpg-web-wasm`'s `applyCommand` does with the guest's encoded command.
        let encoded = JsonProtocol.encode_command(&command).unwrap();
        let decoded = JsonProtocol.decode_command(&encoded).unwrap();
        match self
            .game
            .apply_command(PlayerCommand::new(REMOTE, sequence, decoded).unwrap())
        {
            Ok(()) => Delivery::Applied,
            Err(error) if error.message() == "command sequence is stale" => Delivery::Stale,
            Err(error) => panic!("host rejected the command: {error}"),
        }
    }

    fn advance(&mut self) -> ArpgSnapshot {
        self.game.advance_tick().unwrap();
        self.view()
    }

    fn view(&self) -> ArpgSnapshot {
        let encoded = JsonProtocol
            .encode_snapshot(&self.game.snapshot().unwrap())
            .unwrap();
        JsonProtocol.decode_snapshot(&encoded).unwrap()
    }
}

/// Dedicated server: the pinned `game-server` runtime hosting the ARPG adapter.
struct Dedicated {
    runtime: MatchRuntime<GameServerAdapter<ArpgGame, JsonProtocol>>,
    remote: SessionLease,
}

impl Dedicated {
    fn new() -> Self {
        let adapter = GameServerAdapter::new(arena(), JsonProtocol);
        let mut runtime = MatchRuntime::new(adapter, RECONNECT_GRACE_TICKS);
        let other = runtime.admit(token(7)).unwrap();
        let remote = runtime.admit(token(8)).unwrap();
        assert_eq!((other.player_id, remote.player_id), (1, REMOTE));
        Self { runtime, remote }
    }

    fn submit(
        &mut self,
        connection_epoch: u32,
        sequence: u32,
        command: ArpgCommand,
    ) -> Result<CommandOutcome, RuntimeError> {
        let payload = JsonProtocol.encode_command(&command).unwrap();
        self.runtime
            .submit_command(REMOTE, connection_epoch, sequence, &payload)
    }

    fn disconnect(&mut self) {
        assert!(
            self.runtime
                .disconnect(REMOTE, self.remote.connection_epoch)
        );
    }

    fn reconnect(&mut self, replacement: u8) -> SessionLease {
        let previous = self.remote;
        self.remote = self
            .runtime
            .reconnect(previous.reconnect_token, token(replacement))
            .unwrap();
        assert_eq!(self.remote.player_id, REMOTE);
        assert!(self.remote.connection_epoch > previous.connection_epoch);
        previous
    }
}

impl Authority for Dedicated {
    fn tick(&self) -> u64 {
        self.runtime.current_tick()
    }

    fn deliver(&mut self, sequence: u32, command: ArpgCommand) -> Delivery {
        match self
            .submit(self.remote.connection_epoch, sequence, command)
            .unwrap()
        {
            CommandOutcome::Applied => Delivery::Applied,
            CommandOutcome::IgnoredStale => Delivery::Stale,
        }
    }

    fn advance(&mut self) -> ArpgSnapshot {
        let published = self.runtime.advance_tick().unwrap();
        JsonProtocol.decode_snapshot(&published.payload).unwrap()
    }

    fn view(&self) -> ArpgSnapshot {
        let snapshot = self.runtime.snapshot().unwrap();
        JsonProtocol.decode_snapshot(&snapshot.payload).unwrap()
    }
}

fn token(byte: u8) -> ReconnectToken {
    ReconnectToken([byte; RECONNECT_TOKEN_BYTES])
}

fn remote(snapshot: &ArpgSnapshot) -> &PlayerSnapshot {
    snapshot
        .players
        .iter()
        .find(|player| player.id == REMOTE)
        .unwrap()
}

fn window_ticks() -> u64 {
    base_bundle().counter_window_ticks
}

fn windup_ticks(kind: ActionKind) -> u8 {
    base_bundle()
        .actions
        .iter()
        .find(|action| action.kind == kind)
        .unwrap()
        .windup_ticks
}

/// Ticks between the authority publishing a snapshot and applying a command sent in
/// immediate reply to it, for one-way delay `one_way_ms`: the round trip, which is a whole
/// number of ticks for every representative delay at the 60 Hz tick rate.
fn round_trip_ticks(one_way_ms: u64) -> u64 {
    let scaled = 2 * one_way_ms * u64::from(TICK_HZ);
    assert_eq!(
        scaled % 1000,
        0,
        "{one_way_ms} ms is not a whole tick round trip"
    );
    scaled / 1000
}

/// One remote session against one authority, checking on every published snapshot that
/// counter opportunities appear only from successful blocks and are spent at most once.
struct Session<A> {
    authority: A,
    next_sequence: u32,
    /// The opportunity as last published.
    published: Option<CounterOpportunity>,
    /// Every opportunity a counter action spent, by the tick its block resolved.
    spent: Vec<u64>,
    /// Counter strikes the remote player resolved.
    counter_strikes: usize,
}

impl<A: Authority> Session<A> {
    /// Raises the remote guard and runs until the first block grants an opportunity. Returns
    /// the session at the tick of the snapshot that first publishes it.
    fn blocked(authority: A) -> (Self, CounterOpportunity) {
        let mut session = Self {
            authority,
            next_sequence: 1,
            published: None,
            spent: Vec::new(),
            counter_strikes: 0,
        };
        assert_eq!(
            session.send(ArpgCommand::SetGuard { raised: true }),
            Delivery::Applied
        );
        for _ in 0..120 {
            session.advance();
            if let Some(opportunity) = session.published {
                let tick = session.authority.tick();
                // The first command the authority applies after the block may already use it.
                assert_eq!(opportunity.usable_from_tick, tick);
                assert_eq!(opportunity.blocked_at_tick + 1, tick);
                assert_eq!(
                    opportunity.expires_at_tick,
                    opportunity.usable_from_tick + window_ticks()
                );
                return (session, opportunity);
            }
        }
        panic!("the remote player never blocked");
    }

    /// Sends the remote player's next command, delivered now.
    fn send(&mut self, command: ArpgCommand) -> Delivery {
        let sequence = self.next_sequence;
        self.next_sequence += 1;
        self.deliver(sequence, command)
    }

    /// Delivers a command with an explicit sequence (resends, reordering, duplicates).
    fn deliver(&mut self, sequence: u32, command: ArpgCommand) -> Delivery {
        let before = self.authority.view();
        let delivery = self.authority.deliver(sequence, command);
        let after = self.authority.view();
        // A command never moves the authority's clock: it counts at the tick it arrives.
        assert_eq!(after.tick, before.tick, "the authority rewound or advanced");
        let (was, now) = (remote(&before), remote(&after));
        if delivery == Delivery::Stale {
            assert_eq!(was, now, "a stale command changed the remote player");
        }
        let started = now
            .action
            .filter(|action| was.action.is_none() && action.kind == ActionKind::Counter);
        if let Some(action) = started {
            // The counter starts fresh at the arrival tick and spends an opportunity that was
            // usable at exactly that tick, and never one already spent.
            assert_eq!(action.phase, ActionPhase::Windup);
            assert_eq!(action.ticks_remaining, windup_ticks(ActionKind::Counter));
            let opportunity = was
                .counter
                .expect("a counter started without an opportunity");
            assert!(
                (opportunity.usable_from_tick..opportunity.expires_at_tick).contains(&after.tick)
            );
            assert!(
                !self.spent.contains(&opportunity.blocked_at_tick),
                "one block paid for a second counter"
            );
            self.spent.push(opportunity.blocked_at_tick);
            assert_eq!(now.counter, None, "starting the counter consumes it");
        }
        if was.counter != now.counter {
            assert!(
                started.is_some(),
                "only a started counter changes the opportunity"
            );
        }
        self.published = now.counter;
        delivery
    }

    fn advance(&mut self) -> ArpgSnapshot {
        let snapshot = self.authority.advance();
        let player = remote(&snapshot);
        if let Some(opportunity) = player.counter
            && self.published != Some(opportunity)
        {
            // A new opportunity only ever comes from a block resolved in that very tick.
            assert!(
                snapshot.strike_events.iter().any(|event| {
                    event.target == StrikeTarget::Player(REMOTE)
                        && event.strike_tick == opportunity.blocked_at_tick
                        && matches!(event.result, StrikeResult::Blocked { .. })
                }),
                "an opportunity appeared without a block: {opportunity:?}"
            );
            assert_eq!(opportunity.blocked_at_tick + 1, snapshot.tick);
            assert_eq!(
                opportunity.expires_at_tick,
                opportunity.usable_from_tick + window_ticks(),
                "the window was stretched"
            );
        }
        if let Some(previous) = self.published
            && player.counter != Some(previous)
            && player.counter.is_none()
        {
            // Without a new block an opportunity only lapses at its own expiry, or is lost
            // to a hit (never by latency).
            let hit = snapshot.strike_events.iter().any(|event| {
                event.target == StrikeTarget::Player(REMOTE)
                    && !matches!(event.result, StrikeResult::Blocked { .. })
            });
            assert!(
                hit || snapshot.tick == previous.expires_at_tick,
                "opportunity {previous:?} vanished at tick {}",
                snapshot.tick
            );
        }
        self.counter_strikes += snapshot
            .strike_events
            .iter()
            .filter(|event| {
                event.source == StrikeSource::Player(REMOTE)
                    && event.definition == "sword.counterSlash"
            })
            .count();
        self.published = player.counter;
        snapshot
    }

    fn advance_to(&mut self, tick: u64) {
        assert!(self.authority.tick() <= tick, "no rewinding to tick {tick}");
        while self.authority.tick() < tick {
            self.advance();
        }
    }

    /// The remote player's current action kind, as clients see it.
    fn action(&self) -> Option<ActionKind> {
        remote(&self.authority.view())
            .action
            .map(|action| action.kind)
    }

    fn counter(&self) -> Option<CounterOpportunity> {
        remote(&self.authority.view()).counter
    }

    /// Runs until the remote player's action has finished.
    fn finish_action(&mut self) {
        for _ in 0..120 {
            if self.action().is_none() {
                return;
            }
            self.advance();
        }
        panic!("the remote action never finished");
    }
}

/// Outcome of one remote primary attack applied `round_trip + reaction` ticks after the
/// opportunity was published, and the tick the authority applied it at.
fn remote_primary_attack<A: Authority>(
    authority: A,
    round_trip: u64,
    reaction: u64,
) -> (ActionKind, u64, CounterOpportunity) {
    let (mut session, opportunity) = Session::blocked(authority);
    let apply_tick = opportunity.usable_from_tick + round_trip + reaction;
    session.advance_to(apply_tick);
    // The opportunity reaching the client is the one the authority still holds: delay
    // neither moved its expiry nor kept it past it.
    let held = session.counter();
    if apply_tick < opportunity.expires_at_tick {
        assert_eq!(held, Some(opportunity));
    } else {
        assert_eq!(held, None, "the opportunity outlived its expiry");
    }
    assert_eq!(
        session.send(ArpgCommand::PrimaryAttack { aim: None }),
        Delivery::Applied
    );
    let kind = session
        .action()
        .expect("the primary attack started an action");
    let started = remote(&session.authority.view()).action.unwrap();
    assert_eq!(
        started.phase,
        ActionPhase::Windup,
        "applied as of an earlier tick"
    );
    assert_eq!(started.ticks_remaining, windup_ticks(kind));
    assert_eq!(session.counter(), None);
    (kind, apply_tick, opportunity)
}

#[derive(Clone, Copy, Debug)]
enum Topology {
    PeerHosted,
    Dedicated,
}

const TOPOLOGIES: [Topology; 2] = [Topology::PeerHosted, Topology::Dedicated];

fn attack_under(
    topology: Topology,
    round_trip: u64,
    reaction: u64,
) -> (ActionKind, u64, CounterOpportunity) {
    match topology {
        Topology::PeerHosted => remote_primary_attack(PeerHost::new(), round_trip, reaction),
        Topology::Dedicated => remote_primary_attack(Dedicated::new(), round_trip, reaction),
    }
}

/// The window the tests measure is the content's 30 ticks at 60 Hz.
#[test]
fn the_measured_window_is_the_content_window_at_the_tick_rate() {
    assert_eq!(TICK_HZ, 60);
    assert_eq!(window_ticks(), 30);
    let tuning = arena().tuning_values();
    assert!(tuning.iter().any(|value| {
        value.parameter == TuningParameter::CounterWindowTicks && value.value == 30
    }));
}

/// Claim 1: A remote counter succeeds exactly when the authority applies it inside the window,
/// so a remote player keeps `window - round_trip` ticks to react. Below the boundary the
/// primary attack is the counter; from the boundary on it is an ordinary attack.
#[test]
fn a_remote_counter_succeeds_iff_its_apply_tick_is_inside_the_window() {
    let window = window_ticks();
    // (one-way ms, round-trip ticks, remaining reaction ticks) measured per topology.
    let mut measured = Vec::new();
    for topology in TOPOLOGIES {
        for one_way_ms in ONE_WAY_DELAYS_MS {
            let round_trip = round_trip_ticks(one_way_ms);
            let mut remaining = None;
            // Sweep every reaction from instant to past the whole window.
            for reaction in 0..=window + 1 {
                let (kind, apply_tick, opportunity) = attack_under(topology, round_trip, reaction);
                let inside = (opportunity.usable_from_tick..opportunity.expires_at_tick)
                    .contains(&apply_tick);
                let expected = if inside {
                    ActionKind::Counter
                } else {
                    ActionKind::PrimaryAttack
                };
                assert_eq!(
                    kind, expected,
                    "{topology:?} {one_way_ms} ms, reaction {reaction}: applied at {apply_tick}"
                );
                match (kind, remaining) {
                    (ActionKind::Counter, Some(_)) => {
                        panic!("{topology:?} {one_way_ms} ms: counter after the boundary")
                    }
                    (ActionKind::PrimaryAttack, None) => remaining = Some(reaction),
                    _ => {}
                }
            }
            measured.push((topology, one_way_ms, round_trip, remaining.unwrap_or(0)));
        }
    }
    // The table reported in docs/CONTENT.md: identical for both topologies.
    let expected = [
        (0, 0, 30),
        (25, 3, 27),
        (50, 6, 24),
        (75, 9, 21),
        (100, 12, 18),
    ];
    for (topology, one_way_ms, round_trip, remaining) in measured {
        println!(
            "{topology:?}: one-way {one_way_ms} ms, round trip {round_trip} ticks, \
             reaction window {remaining} ticks ({} ms)",
            remaining * 1000 / u64::from(TICK_HZ)
        );
        assert_eq!(remaining, window - round_trip);
        assert!(expected.contains(&(one_way_ms, round_trip, remaining)));
    }
}

/// Claim 2: Delay never grants extra time. A client that judges the window by the expiry in the
/// snapshot it received (without allowing for the return trip) sends on its last
/// believed-valid tick; the command arrives at or after expiry and is an ordinary attack.
/// Commands carry no tick, so the authority cannot be asked to judge them as of the past.
#[test]
fn delay_never_grants_extra_counter_time() {
    let window = window_ticks();
    for topology in TOPOLOGIES {
        for one_way_ms in ONE_WAY_DELAYS_MS.into_iter().filter(|&ms| ms > 0) {
            let round_trip = round_trip_ticks(one_way_ms);
            // The client reacts on what it believes is the window's last tick.
            let believed_last = window - 1;
            let (kind, apply_tick, opportunity) = attack_under(topology, round_trip, believed_last);
            assert!(apply_tick >= opportunity.expires_at_tick);
            assert_eq!(
                kind,
                ActionKind::PrimaryAttack,
                "{topology:?} {one_way_ms} ms"
            );
            // Every arrival from the expiry onward is ordinary, however the delay is spread.
            for late in 0..=round_trip {
                let reaction = window - round_trip + late;
                let (kind, apply_tick, opportunity) = attack_under(topology, round_trip, reaction);
                assert_eq!(apply_tick, opportunity.expires_at_tick + late);
                assert_eq!(
                    kind,
                    ActionKind::PrimaryAttack,
                    "{topology:?} {one_way_ms} ms"
                );
            }
        }
    }
}

/// Claim 3a: A counter command overtaken by a newer-sequence command (datagram reordering) is
/// ignored as stale: it neither starts the counter nor touches the opportunity, which keeps
/// its original expiry. The peer host's ordered channel cannot reorder, but its authority
/// enforces the same watermark.
#[test]
fn a_counter_command_delivered_after_a_newer_one_is_ignored_as_stale() {
    fn run<A: Authority>(authority: A, topology: Topology) {
        let (mut session, opportunity) = Session::blocked(authority);
        let round_trip = round_trip_ticks(50);
        session.advance_to(opportunity.usable_from_tick + round_trip);
        let counter_sequence = session.next_sequence;
        let newer = counter_sequence + 1;
        session.next_sequence += 2;
        // The newer command (stop moving) overtakes the counter command.
        assert_eq!(
            session.deliver(newer, ArpgCommand::SetMovement { x: 0, z: 0 }),
            Delivery::Applied
        );
        session.advance();
        assert_eq!(
            session.deliver(counter_sequence, ArpgCommand::PrimaryAttack { aim: None }),
            Delivery::Stale,
            "{topology:?}"
        );
        assert_eq!(session.action(), None, "{topology:?}");
        assert_eq!(session.counter(), Some(opportunity), "{topology:?}");
        // Expiry still applies on the original schedule.
        session.advance_to(opportunity.expires_at_tick);
        assert_eq!(session.counter(), None, "{topology:?}");
        assert_eq!(
            session.send(ArpgCommand::PrimaryAttack { aim: None }),
            Delivery::Applied
        );
        assert_eq!(session.action(), Some(ActionKind::PrimaryAttack));
        assert_eq!(session.counter_strikes, 0);
    }
    run(PeerHost::new(), Topology::PeerHosted);
    run(Dedicated::new(), Topology::Dedicated);
}

/// Claim 3b: A duplicated counter command (datagram duplication, or a resend) cannot spend a
/// second counter, neither at once nor after the first counter finished inside the window.
#[test]
fn a_duplicated_counter_command_spends_one_counter_only() {
    fn run<A: Authority>(authority: A, topology: Topology) {
        let (mut session, opportunity) = Session::blocked(authority);
        session.advance_to(opportunity.usable_from_tick + round_trip_ticks(25));
        let sequence = session.next_sequence;
        session.next_sequence += 1;
        assert_eq!(
            session.deliver(sequence, ArpgCommand::PrimaryAttack { aim: None }),
            Delivery::Applied
        );
        assert_eq!(session.action(), Some(ActionKind::Counter), "{topology:?}");
        assert_eq!(
            session.deliver(sequence, ArpgCommand::PrimaryAttack { aim: None }),
            Delivery::Stale,
            "{topology:?}"
        );
        session.finish_action();
        assert!(
            session.authority.tick() < opportunity.expires_at_tick,
            "the duplicate must arrive inside the original window"
        );
        assert_eq!(
            session.deliver(sequence, ArpgCommand::PrimaryAttack { aim: None }),
            Delivery::Stale,
            "{topology:?}"
        );
        assert_eq!(session.action(), None);
        // A fresh attack inside the original window is ordinary: the opportunity is spent.
        assert_eq!(
            session.send(ArpgCommand::PrimaryAttack { aim: None }),
            Delivery::Applied
        );
        assert_eq!(
            session.action(),
            Some(ActionKind::PrimaryAttack),
            "{topology:?}"
        );
        session.finish_action();
        assert_eq!(session.counter_strikes, 1, "{topology:?}");
        assert_eq!(session.spent, vec![opportunity.blocked_at_tick]);
    }
    run(PeerHost::new(), Topology::PeerHosted);
    run(Dedicated::new(), Topology::Dedicated);
}

/// Claim 4a: Disconnecting and reconnecting inside the grace period leaves the opportunity
/// exactly as it was: still usable until the original expiry, never beyond it.
#[test]
fn a_reconnect_does_not_extend_the_counter_window() {
    let window = window_ticks();
    // (disconnect, reconnect, apply) as offsets from `usable_from_tick`.
    for (disconnect, reconnect, apply, expected) in [
        (2, 10, window - 1, Some(ActionKind::Counter)),
        (2, 10, window, Some(ActionKind::PrimaryAttack)),
        // Offline across the expiry: the opportunity is gone on return.
        (2, window + 5, window + 5, Some(ActionKind::PrimaryAttack)),
    ] {
        let (mut session, opportunity) = Session::blocked(Dedicated::new());
        let base = opportunity.usable_from_tick;
        session.advance_to(base + disconnect);
        session.authority.disconnect();
        session.advance_to(base + reconnect);
        assert!(reconnect - disconnect < RECONNECT_GRACE_TICKS);
        let previous = session.authority.reconnect(9);
        let held = session.counter();
        if base + reconnect < opportunity.expires_at_tick {
            assert_eq!(held, Some(opportunity), "reconnect changed the opportunity");
        } else {
            assert_eq!(held, None, "reconnect revived an expired opportunity");
        }
        // The old connection can no longer submit for the player.
        assert_eq!(
            session.authority.submit(
                previous.connection_epoch,
                session.next_sequence,
                ArpgCommand::PrimaryAttack { aim: None }
            ),
            Err(RuntimeError::StaleConnection)
        );
        session.advance_to(base + apply);
        assert_eq!(
            session.send(ArpgCommand::PrimaryAttack { aim: None }),
            Delivery::Applied
        );
        assert_eq!(
            session.action(),
            expected,
            "disconnect {disconnect}, reconnect {reconnect}, apply {apply}"
        );
    }
}

/// Claim 4b: A counter spent before a disconnect is not restored by reconnecting, its command
/// cannot be replayed on the new connection, and no second counter is granted without a
/// new successful block.
#[test]
fn a_reconnect_does_not_restore_a_spent_counter() {
    let (mut session, opportunity) = Session::blocked(Dedicated::new());
    let counter_sequence = session.next_sequence;
    assert_eq!(
        session.send(ArpgCommand::PrimaryAttack { aim: None }),
        Delivery::Applied
    );
    assert_eq!(session.action(), Some(ActionKind::Counter));
    session.advance();
    session.authority.disconnect();
    session.advance_to(opportunity.usable_from_tick + 5);
    let previous = session.authority.reconnect(9);
    assert_eq!(
        session.counter(),
        None,
        "reconnect restored a spent opportunity"
    );
    // The runtime's watermark survives the reconnect: replaying the counter is stale, from
    // either connection.
    assert_eq!(
        session.deliver(counter_sequence, ArpgCommand::PrimaryAttack { aim: None }),
        Delivery::Stale
    );
    assert_eq!(
        session.authority.submit(
            previous.connection_epoch,
            counter_sequence,
            ArpgCommand::PrimaryAttack { aim: None }
        ),
        Err(RuntimeError::StaleConnection)
    );
    session.finish_action();
    assert!(session.authority.tick() < opportunity.expires_at_tick);
    assert_eq!(
        session.send(ArpgCommand::PrimaryAttack { aim: None }),
        Delivery::Applied
    );
    assert_eq!(
        session.action(),
        Some(ActionKind::PrimaryAttack),
        "a second counter inside the spent window"
    );
    session.finish_action();
    // Guard stays held: any later opportunity must come from a new block, which the session
    // checks on every published snapshot.
    for _ in 0..60 {
        session.advance();
    }
    if let Some(later) = session.counter() {
        assert!(later.blocked_at_tick > opportunity.blocked_at_tick);
    }
    assert_eq!(session.counter_strikes, 1);
    assert_eq!(session.spent, vec![opportunity.blocked_at_tick]);
}
