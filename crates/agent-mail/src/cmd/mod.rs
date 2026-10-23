pub mod claim;
pub mod read;
pub mod send;
pub mod wait;

macro_rules! say {
    ($($arg:tt)*) => {
        $crate::show::out(&format!($($arg)*)).map(|()| std::process::ExitCode::SUCCESS)
    };
}
pub(crate) use say;
