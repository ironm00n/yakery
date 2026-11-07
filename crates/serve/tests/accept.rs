use std::{future::ready, io};

use tokio::time::Instant;

use super::*;

fn errno(n: i32) -> io::Result<u8> {
    Err(io::Error::from_raw_os_error(n))
}

#[tokio::test(start_paused = true)]
async fn each_passing_failure_is_waited_out_and_the_next_connection_taken() {
    let mut answers = [errno(24), errno(103), errno(28), Ok(7)].into_iter();
    let start = Instant::now();
    assert_eq!(accept("a socket", || ready(answers.next().unwrap())).await.unwrap(), 7);
    assert_eq!(start.elapsed(), Duration::from_millis(300));
}

#[tokio::test(start_paused = true)]
async fn a_listener_that_cannot_accept_again_is_given_up_at_once() {
    let wrong_family = || Err(io::Error::from(io::ErrorKind::InvalidInput));
    let gone =
        [(errno(22), Some(22)), (errno(9), Some(9)), (errno(88), Some(88)), (errno(1), Some(1))];
    for (gone, shown) in gone.into_iter().chain([(wrong_family(), None)]) {
        let mut answers = [gone, Ok(7)].into_iter();
        let start = Instant::now();
        let refused = accept("a socket", || ready(answers.next().unwrap())).await.unwrap_err();
        let errno = refused.downcast_ref::<io::Error>().unwrap().raw_os_error();
        assert_eq!(
            (refused.to_string(), errno, start.elapsed()),
            ("accepting on a socket".into(), shown, Duration::ZERO)
        );
    }
}
