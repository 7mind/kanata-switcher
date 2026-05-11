pub(crate) type DynError = Box<dyn std::error::Error + Send + Sync>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DbusSuffixError {
    Empty,
    TooLong { length: usize, limit: usize },
}

impl std::fmt::Display for DbusSuffixError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DbusSuffixError::Empty => {
                write!(f, "dbus suffix must contain at least one character")
            }
            DbusSuffixError::TooLong { length, limit } => write!(
                f,
                "dbus suffix length {} exceeds limit {}",
                length, limit
            ),
        }
    }
}

impl std::error::Error for DbusSuffixError {}
