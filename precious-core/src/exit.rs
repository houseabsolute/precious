use anyhow::Error;

#[derive(Debug)]
pub(crate) enum Exit {
    Ok,
    NoFiles,
    FromActionFailures(String),
    FromError(String),
}

impl From<Error> for Exit {
    fn from(err: Error) -> Self {
        Self::FromError(err.to_string())
    }
}

impl Exit {
    pub(crate) fn status(&self) -> i8 {
        match self {
            Exit::Ok | Exit::NoFiles => 0,
            Exit::FromError(_) => 127,
            Exit::FromActionFailures(_) => 1,
        }
    }

    pub(crate) fn error_message(self) -> String {
        match self {
            Exit::Ok => String::new(),
            Exit::NoFiles => String::from("No files found"),
            Exit::FromError(e) | Exit::FromActionFailures(e) => e,
        }
    }
}
