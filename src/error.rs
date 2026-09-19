use crate::query::QueryError;

pub trait StackError: std::error::Error {
    /// format each error stack
    fn format_stack(&self, layer: usize, buf: &mut Vec<String>);
    /// next error
    fn next(&self) -> Option<&dyn StackError>;

    /// last error
    fn last(&self) -> &dyn StackError
    where
        Self: Sized,
    {
        let Some(mut result) = self.next() else {
            return self;
        };
        while let Some(err) = result.next() {
            result = err;
        }
        result
    }
}

pub(crate) trait StackErrorResult<T, E> {
    fn stacked(self) -> Result<T, E>;
}

pub trait StackErrorExt: StackError {
    fn stack_error(&self) -> Vec<String>
    where
        Self: Sized,
    {
        let mut buf = Vec::new();
        let mut layer = 0;
        let mut current: &dyn StackError = self;

        loop {
            current.format_stack(layer, &mut buf);
            match current.next() {
                Some(next) => {
                    current = next;
                    layer += 1;
                }
                None => break,
            }
        }

        buf
    }
}

impl<E: StackError> StackErrorExt for E {}

#[derive(Debug)]
pub struct Error {
    kind: ErrorKind,
    location: &'static std::panic::Location<'static>,
}

#[derive(Debug)]
pub(crate) enum ErrorKind {
    Io(std::io::Error),
    ProstDecode(prost::DecodeError),
    Json(serde_json::Error),
    Query(QueryError),
    Any(Box<dyn std::error::Error + 'static>),
}

impl Error {
    #[track_caller]
    pub(crate) fn new(kind: ErrorKind) -> Self {
        Self {
            kind,
            location: std::panic::Location::caller(),
        }
    }

    #[track_caller]
    pub(crate) fn any(source: Box<dyn std::error::Error + 'static>) -> Self {
        Self::new(ErrorKind::Any(source))
    }
}

impl From<std::io::Error> for Error {
    #[track_caller]
    fn from(value: std::io::Error) -> Self {
        Self::new(ErrorKind::Io(value))
    }
}

impl From<prost::DecodeError> for Error {
    #[track_caller]
    fn from(value: prost::DecodeError) -> Self {
        Self::new(ErrorKind::ProstDecode(value))
    }
}

impl From<serde_json::Error> for Error {
    #[track_caller]
    fn from(value: serde_json::Error) -> Self {
        Self::new(ErrorKind::Json(value))
    }
}

impl From<QueryError> for Error {
    #[track_caller]
    fn from(value: QueryError) -> Self {
        Self::new(ErrorKind::Query(value))
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.kind {
            ErrorKind::Io(source) => source.fmt(f),
            ErrorKind::ProstDecode(source) => source.fmt(f),
            ErrorKind::Json(source) => source.fmt(f),
            ErrorKind::Any(source) => source.fmt(f),
            ErrorKind::Query(source) => source.fmt(f),
        }
    }
}

impl StackError for Error {
    fn format_stack(&self, layer: usize, buf: &mut Vec<String>) {
        buf.push(format!(
            "{}:{} , at {}:{}",
            layer,
            self,
            self.location.file(),
            self.location.line()
        ));
    }

    fn next(&self) -> Option<&dyn StackError> {
        match &self.kind {
            ErrorKind::Query(source) => Some(source),
            _ => None,
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.kind {
            ErrorKind::Io(source) => Some(source),
            ErrorKind::ProstDecode(source) => Some(source),
            ErrorKind::Json(source) => Some(source),
            ErrorKind::Query(source) => Some(source),
            ErrorKind::Any(source) => Some(source.as_ref()),
        }
    }
}
