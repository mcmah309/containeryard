use std::fmt;

use eros::{ErrorUnion, SendSyncError, TypeSet};

/// An error whose message has been deliberately written for application users.
///
/// Other error types remain developer-only unless they are explicitly recognized by
/// [`format_user_error`]. This prevents operational details from being exposed by accident.
#[derive(Debug)]
pub struct UserError(String);

impl UserError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for UserError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for UserError {}

/// Creates an Eros error with a root message that is safe to show to application users.
pub fn user_error(message: impl Into<String>) -> ErrorUnion {
    eros::error!(UserError::new(message))
}

/// Builds the message shown at the CLI boundary.
///
/// Only explicitly safe root errors and `user_context` frames are included. Ordinary context,
/// underlying errors, paths, command output, and backtraces remain available in the developer
/// diagnostic without leaking into the normal user experience.
pub fn format_user_error<E>(error: &ErrorUnion<E>) -> String
where
    E: TypeSet,
{
    let root_message = safe_root_message(error.inner())
        .unwrap_or("Container Yard could not complete the command.");
    let mut message = root_message.to_owned();

    for frame in error.contexts().filter(|frame| frame.is_user_facing()) {
        message.push('\n');
        message.push_str(&frame.to_string());
    }

    message
}

fn safe_root_message(error: &dyn SendSyncError) -> Option<&str> {
    error
        .as_any()
        .downcast_ref::<UserError>()
        .map(|error| error.0.as_str())
}

#[cfg(test)]
mod tests {
    use super::{format_user_error, user_error};

    #[test]
    fn includes_safe_root_and_user_context() {
        let error = user_error("The module declaration is invalid.")
            .context("module path: /private/cache/module.md")
            .user_context("Check the module entry in yard.yaml.");

        assert_eq!(
            format_user_error(&error),
            "The module declaration is invalid.\nCheck the module entry in yard.yaml."
        );
    }

    #[test]
    fn hides_unrecognized_root_and_developer_context() {
        let error = eros::error!("token=secret")
            .context("read /private/config")
            .user_context("Check the configuration and try again.");
        let message = format_user_error(&error);

        assert_eq!(
            message,
            "Container Yard could not complete the command.\nCheck the configuration and try again."
        );
        assert!(!message.contains("secret"));
        assert!(!message.contains("/private"));
    }
}
