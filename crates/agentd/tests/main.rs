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

#[tokio::test]
#[allow(unsafe_code)]
async fn a_loopback_client_is_its_owner() {
    let listener = TcpListener::bind((Ipv4Addr::new(127, 0, 0, 9), 0)).await.unwrap();
    let _client = TcpStream::connect(listener.local_addr().unwrap()).await.unwrap();
    let server = listener.next().await.unwrap();
    assert_eq!(TcpListener::uid(&server).unwrap(), unsafe { libc::getuid() });
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

/// Alpha's session `id` driven over `child` in the background, with a slot of its own unless given one.
async fn driven(
    state: &Arc<State>,
    id: u64,
    mut child: Child,
    slot: Option<OwnedSemaphorePermit>,
) -> (JoinHandle<()>, mpsc::UnboundedReceiver<Reply>) {
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::unbounded_channel();
    let slot = match slot {
        Some(slot) => slot,
        None => Arc::new(Semaphore::new(1)).acquire_owned().await.unwrap(),
    };
    (tokio::spawn(state.clone().drive("alpha".into(), id, child, stdout, tx, slot)), rx)
}

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
    let (driving, mut rx) =
        driven(&state, 0, sh(r#"printf '\377\n{"type":"result"}\n'"#), None).await;
    driving.await.unwrap();
    let mut replies = Vec::new();
    while let Some(reply) = rx.recv().await {
        replies.push(reply.line());
    }
    assert_eq!(replies, ["{\"event\":{\"type\":\"result\"}}\n", "{\"exited\":{\"code\":0}}\n"]);
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
        let (driving, _rx) = driven(&state, 0, child, slot).await;
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
    let (driving, _rx) = driven(&state, 0, sh("true"), None).await;
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
    let (driving, mut rx) = driven(state, 0, output, None).await;
    rx.recv().await.unwrap();
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
