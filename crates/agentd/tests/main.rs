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
