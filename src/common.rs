use std::fmt::Display;

pub struct UserMessageError {
    pub message: Box<dyn Display + 'static + Send + Sync>,
}

impl UserMessageError {
    pub fn new(message: impl Display + 'static + Send + Sync) -> UserMessageError {
        UserMessageError { message: Box::new(message) }
    }
}

impl std::fmt::Display for UserMessageError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::fmt::Debug for UserMessageError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for UserMessageError {}
