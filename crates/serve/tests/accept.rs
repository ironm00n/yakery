use std::io;

use tokio::time::Instant;

use super::*;

#[tokio::test(start_paused = true)]
async fn each_failed_accept_is_waited_out_and_the_next_connection_taken() {
    let emfile = || Err(io::Error::from_raw_os_error(24));
    let mut answers = [emfile(), emfile(), Ok(7)].into_iter();
    let start = Instant::now();
    assert_eq!(accept(async || answers.next().unwrap()).await, 7);
    assert_eq!(start.elapsed(), Duration::from_millis(200));
}
