
pub struct UserMessageError {
    pub message: String
}

impl UserMessageError {
    pub fn new(message: String) -> UserMessageError {
        UserMessageError { message }
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