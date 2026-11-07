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

fn state(agents: &[(&str, u32)]) -> Arc<State> {
    let paths =
        ["--agents", "--key", "--claude", "--mkdir", "--quota-device"].map(|arg| [arg, UNUSED]);
    let args = Args::try_parse_from(
        ["agentd"].into_iter().chain(paths.into_iter().flatten()).chain(["--sessions", "1"]),
    )
    .unwrap();
    Arc::new(State {
        api: UNUSED.into(),
        key: [0; 32],
        agents: agents.iter().map(|&(name, uid)| (name.to_owned(), uid)).collect(),
        sessions: Mutex::default(),
        slots: Arc::new(Semaphore::new(args.sessions)),
        args,
    })
}

async fn replies(mut rx: mpsc::UnboundedReceiver<Reply>) -> Vec<String> {
    let mut lines = Vec::new();
    while let Some(reply) = rx.recv().await {
        lines.push(reply.line());
    }
    lines
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
    let mut child = Command::new("sh")
        .args(["-c", r#"printf '\377\n{"type":"result"}\n'"#])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::unbounded_channel();
    let slot = Arc::new(Semaphore::new(1)).acquire_owned().await.unwrap();
    state(&[("alpha", 2001)]).drive("alpha".into(), child, stdout, tx, slot).await;
    assert_eq!(
        replies(rx).await,
        ["{\"event\":{\"type\":\"result\"}}\n", "{\"exited\":{\"code\":0}}\n"]
    );
}
