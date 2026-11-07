use tokio::task::JoinHandle;
use tokio::time::timeout;

use super::*;

#[test]
fn reads_proc_net_addresses() {
    assert_eq!(
        proc_net_address("0900007F:01BB"),
        Some(SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 9), 443))
    );
}

/// As agentd's TCP listener looks an owner up: off the runtime, in one of LOOKUPS' turns. No other test takes them.
#[test]
#[allow(unsafe_code)]
fn a_loopback_client_is_its_owner_found_off_the_runtime_in_a_turn() {
    one_blocking_thread().block_on(async {
        let (server, _client) = loopback().await;
        let release = take_the_blocking_thread();
        let lookup = tokio::spawn(async move { TcpListener::uid(&server).await });
        yields(100).await;
        assert!(!lookup.is_finished());
        assert_eq!(LOOKUPS.available_permits(), workers() - 1);
        drop(release);
        assert_eq!(lookup.await.unwrap().unwrap(), unsafe { libc::getuid() });
    });
}

const UNUSED: &str = "/nonexistent";
const LONG: Duration = Duration::from_secs(10);

/// One slot, and no quota device, so a wake that gets as far as starting a session fails there.
fn state(agents: &[(&str, u32)]) -> Arc<State> {
    let paths =
        ["--agents", "--key", "--claude", "--mkdir", "--quota-device"].map(|arg| [arg, UNUSED]);
    let args = Args::try_parse_from(
        ["agentd"].into_iter().chain(paths.into_iter().flatten()).chain(["--sessions", "1"]),
    )
    .unwrap();
    let agents = agents.iter().map(|&(name, uid)| (name.to_owned(), uid)).collect();
    Arc::new(State::new(args, UNUSED.into(), [0; 32], agents))
}

async fn put(state: &State, name: &str, next: Run) {
    let agent = &state.agents[name];
    agent.set(&mut *agent.run.lock().await, next);
}

fn wake(state: &Arc<State>, message: String) -> JoinHandle<Result<Woken>> {
    let state = state.clone();
    tokio::spawn(async move { state.wake("alpha", &message).await })
}

/// Session `id` of a process that never reads its input.
fn unread(id: u64) -> (Child, Session) {
    let mut child =
        Command::new("sleep").arg("600").stdin(Stdio::piped()).kill_on_drop(true).spawn().unwrap();
    let stdin = child.stdin.take().unwrap();
    (child, Session::new(id, stdin))
}

fn sh(script: &str) -> Child {
    let mut command = Command::new("sh");
    command.args(["-c", script]).stdin(Stdio::piped()).stdout(Stdio::piped());
    command.kill_on_drop(true).spawn().unwrap()
}

/// Alpha's session `id` driven over `child` in the background into `relay`, with a slot of its own unless given one.
async fn driven_into(
    relay: &Arc<Relay>,
    state: &Arc<State>,
    id: u64,
    mut child: Child,
    slot: Option<OwnedSemaphorePermit>,
) -> JoinHandle<()> {
    let stdout = child.stdout.take().unwrap();
    let slot = match slot {
        Some(slot) => slot,
        None => Arc::new(Semaphore::new(1)).acquire_owned().await.unwrap(),
    };
    tokio::spawn(state.clone().drive("alpha".into(), id, child, stdout, relay.clone(), slot))
}

/// As agentd drives a session it starts.
async fn driven(
    state: &Arc<State>,
    id: u64,
    child: Child,
    slot: Option<OwnedSemaphorePermit>,
) -> (JoinHandle<()>, Reader) {
    let (relay, reader) = Relay::new(MAX_EVENT);
    (driven_into(&relay, state, id, child, slot).await, reader)
}

/// Everything the relay tells, as lines; a reply that doesn't come fails rather than hangs.
async fn told(reader: &Reader) -> Vec<String> {
    let mut lines = Vec::new();
    while let Some(reply) = timeout(LONG, reader.next()).await.unwrap() {
        lines.push(reply.line());
    }
    lines
}

/// The events a relay holds and their cost, read out before any assert so none panics with its lock held.
fn held(relay: &Relay) -> (usize, usize) {
    let queue = relay.queue();
    (queue.events.len(), queue.cost)
}

/// The relay's budget for `events` events of `bytes` bytes each.
fn budget(events: usize, bytes: usize) -> usize {
    events * (bytes + EVENT_COST)
}

/// Twenty events, `{"n":1}` to `{"n":20}`.
const TWENTY: &str = r#"i=1; while [ $i -le 20 ]; do printf '{"n":%d}\n' $i; i=$((i+1)); done"#;

#[tokio::test]
async fn a_line_is_whole_or_skipped_whole_past_the_bound() {
    for (input, expected) in [
        (b"ab\nabcd\nabcdefgh\ncd\nef".as_slice(), ["ab", "abcd", "over", "cd", "ef"].as_slice()),
        (b"ab\nabcdefgh", &["ab", "over"]),
    ] {
        let mut lines = Lines::new(BufReader::with_capacity(3, input), 4);
        let mut read = Vec::new();
        while let Some(line) = lines.next().await.unwrap() {
            read.push(match line {
                Line::Whole(line) => String::from_utf8(line).unwrap(),
                Line::Over => "over".into(),
            });
        }
        assert_eq!(read, expected);
    }
}

#[tokio::test]
async fn a_session_is_read_on_past_a_line_that_is_not_utf8() {
    let state = state(&[("alpha", 2001)]);
    let (driving, reader) =
        driven(&state, 0, sh(r#"printf '\377\n{"type":"result"}\n'"#), None).await;
    driving.await.unwrap();
    assert_eq!(
        told(&reader).await,
        ["{\"event\":{\"type\":\"result\"}}\n", "{\"exited\":{\"code\":0}}\n"]
    );
}

#[tokio::test]
async fn a_session_that_stops_reading_refuses_wakes_past_its_queue_and_holds_up_no_other() {
    let state = state(&[("alpha", 2001), ("beta", 2002)]);
    let ((_alpha, alpha), (_beta, beta)) = (unread(0), unread(1));
    put(&state, "alpha", Run::Running(alpha)).await;
    put(&state, "beta", Run::Running(beta)).await;
    let past_the_pipe = "x".repeat(1 << 20);
    let mut taken = Vec::new();
    for _ in 0..QUEUED + 2 {
        taken.push(timeout(LONG, state.wake("alpha", &past_the_pipe)).await.unwrap().is_ok());
    }
    // the writer may hold one more than the queue
    assert!(taken[..QUEUED].iter().all(|&taken| taken));
    assert!(taken[QUEUED..].contains(&false));
    assert!(matches!(timeout(LONG, state.wake("beta", "hi")).await, Ok(Ok(Woken::Injected))));
}

#[tokio::test]
async fn a_wake_holds_no_slot_while_another_wake_starts_its_agent() {
    let state = state(&[("alpha", 2001)]);
    let every_slot = state.slots.clone().acquire_owned().await.unwrap();
    let waiting = wake(&state, "hi".into());
    tokio::task::yield_now().await;
    let starting = state.agents["alpha"].run.lock().await;
    drop(every_slot);
    tokio::task::yield_now().await;
    assert_eq!(state.slots.available_permits(), 1);
    drop(starting);
    timeout(LONG, waiting).await.unwrap().unwrap().err().unwrap();
}

#[tokio::test]
async fn a_wake_waiting_for_a_slot_takes_the_session_its_agent_starts_meanwhile() {
    let state = state(&[("alpha", 2001)]);
    let _every_slot = state.slots.clone().acquire_owned().await.unwrap();
    let waiting = wake(&state, "hi".into());
    tokio::task::yield_now().await;
    assert!(!waiting.is_finished());
    let (_alpha, alpha) = unread(0);
    put(&state, "alpha", Run::Running(alpha)).await;
    assert!(matches!(timeout(LONG, waiting).await.unwrap().unwrap(), Ok(Woken::Injected)));
}

/// Also when the ending session holds the last slot, which it can give back only once it has its agent's lock.
#[tokio::test]
async fn a_wake_for_an_ending_session_starts_the_next_once_its_process_exits() {
    for holds_the_last_slot in [false, true] {
        let state = state(&[("alpha", 2001)]);
        let mut child = sh("read line; exec >&-; read line");
        let mut input = child.stdin.take().unwrap();
        put(&state, "alpha", Run::Ending(0)).await;
        let slot = holds_the_last_slot.then(|| state.slots.clone().try_acquire_owned().unwrap());
        let (driving, _relay) = driven(&state, 0, child, slot).await;
        let waiting = wake(&state, "hi".into());
        tokio::task::yield_now().await;
        let changed = state.agents["alpha"].changed.notified();
        let mut changed = std::pin::pin!(changed);
        changed.as_mut().enable();
        input.write_all(b"\n").await.unwrap();
        // its output has ended, its process not
        timeout(LONG, changed).await.unwrap();
        tokio::task::yield_now().await;
        assert!(!waiting.is_finished());
        input.write_all(b"\n").await.unwrap();
        timeout(LONG, driving).await.unwrap().unwrap();
        let started = timeout(LONG, waiting).await.unwrap().unwrap();
        assert!(format!("{:#}", started.err().unwrap()).contains("quotactl"));
    }
}

#[tokio::test]
async fn a_session_that_ends_leaves_the_one_started_after_it() {
    let state = state(&[("alpha", 2001)]);
    let (_next, next) = unread(1);
    put(&state, "alpha", Run::Running(next)).await;
    let (driving, _relay) = driven(&state, 0, sh("true"), None).await;
    driving.await.unwrap();
    assert_eq!(state.agents["alpha"].run.lock().await.id(), Some(1));
}

#[tokio::test(start_paused = true)]
async fn a_wake_no_slot_comes_to_within_the_bound_is_answered_busy() {
    let state = state(&[("alpha", 2001)]);
    let _every_slot = state.slots.clone().acquire_owned().await.unwrap();
    let answered = timeout(FIRST_REPLY * 2, state.wake("alpha", "hi")).await.unwrap();
    assert!(matches!(answered, Ok(Woken::Busy)));
}

/// A start holds the agent's lock across its CA fetch.
#[tokio::test(start_paused = true)]
async fn a_wake_behind_a_start_that_does_not_finish_is_answered_busy() {
    let state = state(&[("alpha", 2001)]);
    let _starting = state.agents["alpha"].run.lock().await;
    let answered = timeout(FIRST_REPLY * 2, state.wake("alpha", "hi")).await.unwrap();
    assert!(matches!(answered, Ok(Woken::Busy)));
}

/// The wake waits past the bound for alpha's last session to exit; a session taking input after that gets nothing.
#[tokio::test(start_paused = true)]
async fn a_wake_answered_busy_is_not_made_later() {
    let state = state(&[("alpha", 2001)]);
    put(&state, "alpha", Run::Ending(0)).await;
    let answered = timeout(FIRST_REPLY * 2, state.wake("alpha", "hi")).await.unwrap();
    assert!(matches!(answered, Ok(Woken::Busy)));
    let (input, mut taken) = mpsc::channel(QUEUED);
    put(&state, "alpha", Run::Running(Session { id: 1, input, injected: Instant::now() })).await;
    tokio::task::yield_now().await;
    assert!(taken.try_recv().is_err());
}

#[tokio::test(start_paused = true)]
async fn a_slot_freed_within_the_bound_is_taken() {
    let state = state(&[("alpha", 2001)]);
    let every_slot = state.slots.clone().acquire_owned().await.unwrap();
    let waiting = wake(&state, "hi".into());
    // its deadline set, so a shorter bound expires while the clock moves
    tokio::task::yield_now().await;
    tokio::time::advance(FIRST_REPLY - Duration::from_secs(1)).await;
    drop(every_slot);
    let started = timeout(LONG, waiting).await.unwrap().unwrap();
    assert!(format!("{:#}", started.err().unwrap()).contains("quotactl"));
}

/// Alpha's session 0, whose turn has just ended, driven in the background.
async fn turn_ended(state: &Arc<State>) -> (Child, JoinHandle<()>) {
    let (input, session) = unread(0);
    put(state, "alpha", Run::Running(session)).await;
    let output = sh(r#"echo '{"type":"result"}'; exec sleep 600"#);
    let (driving, reader) = driven(state, 0, output, None).await;
    // no timeout: on these tests' paused clock it would fire before the output comes
    reader.next().await.unwrap();
    (input, driving)
}

#[tokio::test(start_paused = true)]
async fn a_quiet_period_with_no_wake_ends_the_session() {
    let state = state(&[("alpha", 2001)]);
    let _session = turn_ended(&state).await;
    tokio::time::sleep(QUIET * 2).await;
    assert!(matches!(*state.agents["alpha"].run.lock().await, Run::Ending(0)));
}

#[tokio::test(start_paused = true)]
async fn a_wake_in_the_quiet_period_keeps_the_session() {
    let state = state(&[("alpha", 2001)]);
    let _session = turn_ended(&state).await;
    tokio::time::advance(QUIET / 2).await;
    assert!(matches!(state.wake("alpha", "hi").await, Ok(Woken::Injected)));
    tokio::time::sleep(QUIET * 2).await;
    assert!(matches!(*state.agents["alpha"].run.lock().await, Run::Running(_)));
}

fn one_blocking_thread() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap()
}

/// Takes the one blocking thread until the sender is dropped.
fn take_the_blocking_thread() -> std::sync::mpsc::Sender<()> {
    let (release, held) = std::sync::mpsc::channel::<()>();
    tokio::task::spawn_blocking(move || held.recv());
    release
}

/// A loopback connection's server end, and its client kept open.
async fn loopback() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind((Ipv4Addr::new(127, 0, 0, 9), 0)).await.unwrap();
    let client = TcpStream::connect(listener.local_addr().unwrap()).await.unwrap();
    (listener.next().await.unwrap(), client)
}

async fn yields(n: usize) {
    for _ in 0..n {
        tokio::task::yield_now().await;
    }
}

/// A lookup of `server`'s client in `turns`, in the background.
fn look_up(server: &TcpStream, turns: &Arc<Semaphore>) -> JoinHandle<Result<u32>> {
    let (Ok(SocketAddr::V4(client)), Ok(SocketAddr::V4(server))) =
        (server.peer_addr(), server.local_addr())
    else {
        panic!("a loopback IPv4 connection");
    };
    let turns = turns.clone();
    tokio::spawn(async move { loopback_owner(client, server, &turns).await })
}

/// With the blocking thread taken, a lookup that reads off the runtime can't finish however often it is polled.
#[test]
#[allow(unsafe_code)]
fn a_loopback_owner_is_looked_up_off_the_runtime() {
    one_blocking_thread().block_on(async {
        let (server, _client) = loopback().await;
        let release = take_the_blocking_thread();
        let lookup = look_up(&server, &Arc::new(Semaphore::new(1)));
        yields(100).await;
        assert!(!lookup.is_finished());
        drop(release);
        assert_eq!(lookup.await.unwrap().unwrap(), unsafe { libc::getuid() });
    });
}

/// With every turn held here, a read sent to the blocking thread after the lookup started ends with the lookup still
/// waiting.
#[test]
fn a_loopback_owner_lookup_waits_for_a_turn() {
    one_blocking_thread().block_on(async {
        let (server, _client) = loopback().await;
        let turns = Arc::new(Semaphore::new(1));
        let turn = turns.clone().acquire_owned().await.unwrap();
        let lookup = look_up(&server, &turns);
        yields(1).await;
        tokio::task::spawn_blocking(|| ()).await.unwrap();
        yields(100).await;
        assert!(!lookup.is_finished());
        drop(turn);
        lookup.await.unwrap().unwrap();
    });
}

#[test]
fn a_loopback_owner_lookup_holds_its_turn_until_its_read_ends() {
    one_blocking_thread().block_on(async {
        let (server, _client) = loopback().await;
        let turns = Arc::new(Semaphore::new(1));
        let release = take_the_blocking_thread();
        let lookup = look_up(&server, &turns);
        yields(100).await;
        assert_eq!(turns.available_permits(), 0);
        drop(release);
        lookup.await.unwrap().unwrap();
        assert_eq!(turns.available_permits(), 1);
    });
}

/// Two runs, one after the other, each naming sessions as `start` does.
#[test]
fn a_later_run_names_no_session_as_an_earlier_run_did() {
    let name = |state: &State| unit("alpha", state.started.fetch_add(1, Ordering::Relaxed));
    let earlier = state(&[("alpha", 2001)]);
    let named = [name(&earlier), name(&earlier)];
    assert_ne!(named[0], named[1]);
    assert!(!named.contains(&name(&state(&[("alpha", 2001)]))));
}

/// A waker that reads nothing until the session has exited: the session runs to its end, and the waker then gets the
/// newest events that fit three of 8 bytes, in order, the number dropped, and the exit.
#[tokio::test]
async fn a_waker_that_never_reads_holds_up_no_session() {
    let state = state(&[("alpha", 2001)]);
    let (relay, reader) = Relay::new(budget(3, 8));
    timeout(LONG, driven_into(&relay, &state, 0, sh(TWENTY), None).await).await.unwrap().unwrap();
    assert_eq!(
        told(&reader).await,
        [
            "{\"event\":{\"n\":18}}\n",
            "{\"event\":{\"n\":19}}\n",
            "{\"event\":{\"n\":20}}\n",
            "{\"dropped\":{\"events\":17}}\n",
            "{\"exited\":{\"code\":0}}\n",
        ]
    );
}

#[tokio::test]
async fn the_newest_event_is_kept_even_past_the_budget() {
    let state = state(&[("alpha", 2001)]);
    let (relay, reader) = Relay::new(1);
    timeout(LONG, driven_into(&relay, &state, 0, sh(TWENTY), None).await).await.unwrap().unwrap();
    assert_eq!(
        told(&reader).await,
        [
            "{\"event\":{\"n\":20}}\n",
            "{\"dropped\":{\"events\":19}}\n",
            "{\"exited\":{\"code\":0}}\n"
        ]
    );
}

#[tokio::test]
async fn a_waker_within_the_budget_is_told_every_event() {
    let state = state(&[("alpha", 2001)]);
    let (driving, reader) = driven(&state, 0, sh(TWENTY), None).await;
    timeout(LONG, driving).await.unwrap().unwrap();
    let events = (1..=20).map(|n| format!("{{\"event\":{{\"n\":{n}}}}}\n"));
    let all = events.chain(["{\"exited\":{\"code\":0}}\n".to_owned()]).collect::<Vec<_>>();
    assert_eq!(told(&reader).await, all);
}

/// Three events of 7 bytes fit, are read, and then three more fit again.
#[tokio::test]
async fn events_the_waker_has_read_free_their_cost() {
    let state = state(&[("alpha", 2001)]);
    let (relay, reader) = Relay::new(budget(3, 7));
    let mut child = sh(
        r#"printf '{"n":1}\n{"n":2}\n{"n":3}\n'; read line; printf '{"n":4}\n{"n":5}\n{"n":6}\n'"#,
    );
    let mut input = child.stdin.take().unwrap();
    let driving = driven_into(&relay, &state, 0, child, None).await;
    let mut read = Vec::new();
    for _ in 0..3 {
        read.push(timeout(LONG, reader.next()).await.unwrap().unwrap().line());
    }
    input.write_all(b"\n").await.unwrap();
    timeout(LONG, driving).await.unwrap().unwrap();
    read.extend(told(&reader).await);
    let events = (1..=6).map(|n| format!("{{\"event\":{{\"n\":{n}}}}}\n"));
    assert_eq!(read, events.chain(["{\"exited\":{\"code\":0}}\n".to_owned()]).collect::<Vec<_>>());
}

/// The reader is waiting when the session, which says nothing, exits.
#[tokio::test]
async fn a_waiting_waker_is_told_the_exit() {
    let state = state(&[("alpha", 2001)]);
    let (relay, reader) = Relay::new(MAX_EVENT);
    let waiting = tokio::spawn(async move { reader.next().await.map(|reply| reply.line()) });
    tokio::task::yield_now().await;
    timeout(LONG, driven_into(&relay, &state, 0, sh("exit 7"), None).await).await.unwrap().unwrap();
    let told = timeout(LONG, waiting).await.unwrap().unwrap();
    assert_eq!(told.as_deref(), Some("{\"exited\":{\"code\":7}}\n"));
}

#[tokio::test]
async fn a_relay_whose_waker_is_gone_holds_nothing() {
    let state = state(&[("alpha", 2001)]);
    let (relay, reader) = Relay::new(MAX_EVENT);
    drop(reader);
    timeout(LONG, driven_into(&relay, &state, 0, sh(TWENTY), None).await).await.unwrap().unwrap();
    assert_eq!(held(&relay), (0, 0));
}

/// Twenty events wait unread when the waker goes.
#[tokio::test]
async fn a_relay_lets_go_of_what_it_held_when_its_waker_goes() {
    let state = state(&[("alpha", 2001)]);
    let (relay, reader) = Relay::new(MAX_EVENT);
    timeout(LONG, driven_into(&relay, &state, 0, sh(TWENTY), None).await).await.unwrap().unwrap();
    assert_eq!(held(&relay).0, 20);
    drop(reader);
    assert_eq!(held(&relay), (0, 0));
}

/// A thousand one-byte events, then one the size of the budget, which drops them.
#[test]
fn a_relay_gives_back_the_room_of_events_it_dropped() {
    let (relay, _reader) = Relay::new(budget(1000, 1));
    for _ in 0..1000 {
        relay.event(RawValue::from_string("1".into()).unwrap());
    }
    relay.event(RawValue::from_string(format!("\"{}\"", "x".repeat(budget(1000, 1)))).unwrap());
    let (events, room) = {
        let queue = relay.queue();
        (queue.events.len(), queue.events.capacity())
    };
    assert_eq!(events, 1);
    assert!(room <= 64, "{room}");
}

/// A waker that fell behind a burst of a thousand one-byte events, and then read them all.
#[tokio::test]
async fn a_relay_gives_back_the_room_of_events_it_has_told() {
    let (relay, reader) = Relay::new(budget(1000, 1));
    for _ in 0..1000 {
        relay.event(RawValue::from_string("1".into()).unwrap());
    }
    for _ in 0..1000 {
        timeout(LONG, reader.next()).await.unwrap().unwrap();
    }
    let room = relay.queue().events.capacity();
    assert!(room <= 64, "{room}");
}
