pub mod dns;
pub mod process;

pub type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;
