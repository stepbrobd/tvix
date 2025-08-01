use std::collections::VecDeque;
use std::fmt;
use std::io;
use std::thread;

#[cfg(test)]
use ::proptest::prelude::TestCaseError;
use thiserror::Error;

use crate::wire::ProtocolVersion;

use super::NixWrite;

#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum Error {
    #[error("custom error '{0}'")]
    Custom(String),
    #[error("unsupported data error '{0}'")]
    UnsupportedData(String),
    #[error("Invalid enum: {0}")]
    InvalidEnum(String),
    #[error("IO error {0} '{1}'")]
    IO(io::ErrorKind, String),
    #[error("wrong write: expected {0} got {1}")]
    WrongWrite(OperationType, OperationType),
    #[error("unexpected write: got an extra {0}")]
    ExtraWrite(OperationType),
    #[error("got an unexpected number {0} in write_number")]
    UnexpectedNumber(u64),
    #[error("got an unexpected slice '{0:?}' in write_slice")]
    UnexpectedSlice(Vec<u8>),
    #[error("got an unexpected display '{0:?}' in write_slice")]
    UnexpectedDisplay(String),
}

impl Error {
    pub fn unexpected_write_number(expected: OperationType) -> Error {
        Error::WrongWrite(expected, OperationType::WriteNumber)
    }

    pub fn extra_write_number() -> Error {
        Error::ExtraWrite(OperationType::WriteNumber)
    }

    pub fn unexpected_write_slice(expected: OperationType) -> Error {
        Error::WrongWrite(expected, OperationType::WriteSlice)
    }

    pub fn unexpected_write_display(expected: OperationType) -> Error {
        Error::WrongWrite(expected, OperationType::WriteDisplay)
    }
}

impl super::Error for Error {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        Self::Custom(msg.to_string())
    }

    fn io_error(err: std::io::Error) -> Self {
        Self::IO(err.kind(), err.to_string())
    }

    fn unsupported_data<T: fmt::Display>(msg: T) -> Self {
        Self::UnsupportedData(msg.to_string())
    }

    fn invalid_enum<T: fmt::Display>(msg: T) -> Self {
        Self::InvalidEnum(msg.to_string())
    }
}

#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OperationType {
    WriteNumber,
    WriteSlice,
    WriteDisplay,
}

impl fmt::Display for OperationType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WriteNumber => write!(f, "write_number"),
            Self::WriteSlice => write!(f, "write_slice"),
            Self::WriteDisplay => write!(f, "write_display"),
        }
    }
}

#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, PartialEq, Eq)]
enum Operation {
    WriteNumber(u64, Result<(), Error>),
    WriteSlice(Vec<u8>, Result<(), Error>),
    WriteDisplay(String, Result<(), Error>),
}

impl From<Operation> for OperationType {
    fn from(value: Operation) -> Self {
        match value {
            Operation::WriteNumber(_, _) => OperationType::WriteNumber,
            Operation::WriteSlice(_, _) => OperationType::WriteSlice,
            Operation::WriteDisplay(_, _) => OperationType::WriteDisplay,
        }
    }
}

pub struct Builder {
    version: ProtocolVersion,
    ops: VecDeque<Operation>,
}

impl Builder {
    pub fn new() -> Builder {
        Builder {
            version: Default::default(),
            ops: VecDeque::new(),
        }
    }

    pub fn version<V: Into<ProtocolVersion>>(&mut self, version: V) -> &mut Self {
        self.version = version.into();
        self
    }

    pub fn write_number(&mut self, value: u64) -> &mut Self {
        self.ops.push_back(Operation::WriteNumber(value, Ok(())));
        self
    }

    pub fn write_number_error(&mut self, value: u64, err: Error) -> &mut Self {
        self.ops.push_back(Operation::WriteNumber(value, Err(err)));
        self
    }

    pub fn write_slice(&mut self, value: &[u8]) -> &mut Self {
        self.ops
            .push_back(Operation::WriteSlice(value.to_vec(), Ok(())));
        self
    }

    pub fn write_slice_error(&mut self, value: &[u8], err: Error) -> &mut Self {
        self.ops
            .push_back(Operation::WriteSlice(value.to_vec(), Err(err)));
        self
    }

    pub fn write_display<D>(&mut self, value: D) -> &mut Self
    where
        D: fmt::Display,
    {
        let msg = value.to_string();
        self.ops.push_back(Operation::WriteDisplay(msg, Ok(())));
        self
    }

    pub fn write_display_error<D>(&mut self, value: D, err: Error) -> &mut Self
    where
        D: fmt::Display,
    {
        let msg = value.to_string();
        self.ops.push_back(Operation::WriteDisplay(msg, Err(err)));
        self
    }

    #[cfg(test)]
    fn write_operation_type(&mut self, op: OperationType) -> &mut Self {
        match op {
            OperationType::WriteNumber => self.write_number(10),
            OperationType::WriteSlice => self.write_slice(b"testing"),
            OperationType::WriteDisplay => self.write_display("testing"),
        }
    }

    #[cfg(test)]
    fn write_operation(&mut self, op: &Operation) -> &mut Self {
        match op {
            Operation::WriteNumber(value, Ok(_)) => self.write_number(*value),
            Operation::WriteNumber(value, Err(Error::UnexpectedNumber(_))) => {
                self.write_number(*value)
            }
            Operation::WriteNumber(_, Err(Error::ExtraWrite(OperationType::WriteNumber))) => self,
            Operation::WriteNumber(_, Err(Error::WrongWrite(op, OperationType::WriteNumber))) => {
                self.write_operation_type(*op)
            }
            Operation::WriteNumber(value, Err(Error::Custom(msg))) => {
                self.write_number_error(*value, Error::Custom(msg.clone()))
            }
            Operation::WriteNumber(value, Err(Error::IO(kind, msg))) => {
                self.write_number_error(*value, Error::IO(*kind, msg.clone()))
            }
            Operation::WriteSlice(value, Ok(_)) => self.write_slice(value),
            Operation::WriteSlice(value, Err(Error::UnexpectedSlice(_))) => self.write_slice(value),
            Operation::WriteSlice(_, Err(Error::ExtraWrite(OperationType::WriteSlice))) => self,
            Operation::WriteSlice(_, Err(Error::WrongWrite(op, OperationType::WriteSlice))) => {
                self.write_operation_type(*op)
            }
            Operation::WriteSlice(value, Err(Error::Custom(msg))) => {
                self.write_slice_error(value, Error::Custom(msg.clone()))
            }
            Operation::WriteSlice(value, Err(Error::IO(kind, msg))) => {
                self.write_slice_error(value, Error::IO(*kind, msg.clone()))
            }
            Operation::WriteDisplay(value, Ok(_)) => self.write_display(value),
            Operation::WriteDisplay(value, Err(Error::Custom(msg))) => {
                self.write_display_error(value, Error::Custom(msg.clone()))
            }
            Operation::WriteDisplay(value, Err(Error::IO(kind, msg))) => {
                self.write_display_error(value, Error::IO(*kind, msg.clone()))
            }
            Operation::WriteDisplay(value, Err(Error::UnexpectedDisplay(_))) => {
                self.write_display(value)
            }
            Operation::WriteDisplay(_, Err(Error::ExtraWrite(OperationType::WriteDisplay))) => self,
            Operation::WriteDisplay(_, Err(Error::WrongWrite(op, OperationType::WriteDisplay))) => {
                self.write_operation_type(*op)
            }
            s => panic!("Invalid operation {s:?}"),
        }
    }

    pub fn build(&mut self) -> Mock {
        Mock {
            version: self.version,
            ops: self.ops.clone(),
        }
    }
}

impl Default for Builder {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Mock {
    version: ProtocolVersion,
    ops: VecDeque<Operation>,
}

impl Mock {
    #[cfg(test)]
    #[allow(dead_code)]
    async fn assert_operation(&mut self, op: Operation) {
        match op {
            Operation::WriteNumber(_, Err(Error::UnexpectedNumber(value))) => {
                assert_eq!(
                    self.write_number(value).await,
                    Err(Error::UnexpectedNumber(value))
                );
            }
            Operation::WriteNumber(value, res) => {
                assert_eq!(self.write_number(value).await, res);
            }
            Operation::WriteSlice(_, ref res @ Err(Error::UnexpectedSlice(ref value))) => {
                assert_eq!(self.write_slice(value).await, res.clone());
            }
            Operation::WriteSlice(value, res) => {
                assert_eq!(self.write_slice(&value).await, res);
            }
            Operation::WriteDisplay(_, ref res @ Err(Error::UnexpectedDisplay(ref value))) => {
                assert_eq!(self.write_display(value).await, res.clone());
            }
            Operation::WriteDisplay(value, res) => {
                assert_eq!(self.write_display(value).await, res);
            }
        }
    }

    #[cfg(test)]
    async fn prop_assert_operation(&mut self, op: Operation) -> Result<(), TestCaseError> {
        use ::proptest::prop_assert_eq;

        match op {
            Operation::WriteNumber(_, Err(Error::UnexpectedNumber(value))) => {
                prop_assert_eq!(
                    self.write_number(value).await,
                    Err(Error::UnexpectedNumber(value))
                );
            }
            Operation::WriteNumber(value, res) => {
                prop_assert_eq!(self.write_number(value).await, res);
            }
            Operation::WriteSlice(_, ref res @ Err(Error::UnexpectedSlice(ref value))) => {
                prop_assert_eq!(self.write_slice(value).await, res.clone());
            }
            Operation::WriteSlice(value, res) => {
                prop_assert_eq!(self.write_slice(&value).await, res);
            }
            Operation::WriteDisplay(_, ref res @ Err(Error::UnexpectedDisplay(ref value))) => {
                prop_assert_eq!(self.write_display(&value).await, res.clone());
            }
            Operation::WriteDisplay(value, res) => {
                prop_assert_eq!(self.write_display(&value).await, res);
            }
        }
        Ok(())
    }
}

impl NixWrite for Mock {
    type Error = Error;

    fn version(&self) -> ProtocolVersion {
        self.version
    }

    async fn write_number(&mut self, value: u64) -> Result<(), Self::Error> {
        match self.ops.pop_front() {
            Some(Operation::WriteNumber(expected, ret)) => {
                if value != expected {
                    return Err(Error::UnexpectedNumber(value));
                }
                ret
            }
            Some(op) => Err(Error::unexpected_write_number(op.into())),
            _ => Err(Error::ExtraWrite(OperationType::WriteNumber)),
        }
    }

    async fn write_slice(&mut self, buf: &[u8]) -> Result<(), Self::Error> {
        match self.ops.pop_front() {
            Some(Operation::WriteSlice(expected, ret)) => {
                if buf != expected {
                    return Err(Error::UnexpectedSlice(buf.to_vec()));
                }
                ret
            }
            Some(op) => Err(Error::unexpected_write_slice(op.into())),
            _ => Err(Error::ExtraWrite(OperationType::WriteSlice)),
        }
    }

    async fn write_display<D>(&mut self, msg: D) -> Result<(), Self::Error>
    where
        D: fmt::Display + Send,
        Self: Sized,
    {
        let value = msg.to_string();
        match self.ops.pop_front() {
            Some(Operation::WriteDisplay(expected, ret)) => {
                if value != expected {
                    return Err(Error::UnexpectedDisplay(value));
                }
                ret
            }
            Some(op) => Err(Error::unexpected_write_display(op.into())),
            _ => Err(Error::ExtraWrite(OperationType::WriteDisplay)),
        }
    }
}

impl Drop for Mock {
    fn drop(&mut self) {
        // No need to panic again
        if thread::panicking() {
            return;
        }
        if let Some(op) = self.ops.front() {
            panic!("reader dropped with {op:?} operation still unread")
        }
    }
}

#[cfg(test)]
mod proptest {
    use std::io;

    use proptest::{
        prelude::{any, Arbitrary, BoxedStrategy, Just, Strategy},
        prop_oneof,
    };

    use super::{Error, Operation, OperationType};

    pub fn arb_write_number_operation() -> impl Strategy<Value = Operation> {
        (
            any::<u64>(),
            prop_oneof![
                Just(Ok(())),
                any::<u64>().prop_map(|v| Err(Error::UnexpectedNumber(v))),
                Just(Err(Error::WrongWrite(
                    OperationType::WriteSlice,
                    OperationType::WriteNumber
                ))),
                Just(Err(Error::WrongWrite(
                    OperationType::WriteDisplay,
                    OperationType::WriteNumber
                ))),
                any::<String>().prop_map(|s| Err(Error::Custom(s))),
                (any::<io::ErrorKind>(), any::<String>())
                    .prop_map(|(kind, msg)| Err(Error::IO(kind, msg))),
            ],
        )
            .prop_filter("same number", |(v, res)| match res {
                Err(Error::UnexpectedNumber(exp_v)) => v != exp_v,
                _ => true,
            })
            .prop_map(|(v, res)| Operation::WriteNumber(v, res))
    }

    pub fn arb_write_slice_operation() -> impl Strategy<Value = Operation> {
        (
            any::<Vec<u8>>(),
            prop_oneof![
                Just(Ok(())),
                any::<Vec<u8>>().prop_map(|v| Err(Error::UnexpectedSlice(v))),
                Just(Err(Error::WrongWrite(
                    OperationType::WriteNumber,
                    OperationType::WriteSlice
                ))),
                Just(Err(Error::WrongWrite(
                    OperationType::WriteDisplay,
                    OperationType::WriteSlice
                ))),
                any::<String>().prop_map(|s| Err(Error::Custom(s))),
                (any::<io::ErrorKind>(), any::<String>())
                    .prop_map(|(kind, msg)| Err(Error::IO(kind, msg))),
            ],
        )
            .prop_filter("same slice", |(v, res)| match res {
                Err(Error::UnexpectedSlice(exp_v)) => v != exp_v,
                _ => true,
            })
            .prop_map(|(v, res)| Operation::WriteSlice(v, res))
    }

    #[allow(dead_code)]
    pub fn arb_extra_write() -> impl Strategy<Value = Operation> {
        prop_oneof![
            any::<u64>().prop_map(|msg| {
                Operation::WriteNumber(msg, Err(Error::ExtraWrite(OperationType::WriteNumber)))
            }),
            any::<Vec<u8>>().prop_map(|msg| {
                Operation::WriteSlice(msg, Err(Error::ExtraWrite(OperationType::WriteSlice)))
            }),
            any::<String>().prop_map(|msg| {
                Operation::WriteDisplay(msg, Err(Error::ExtraWrite(OperationType::WriteDisplay)))
            }),
        ]
    }

    pub fn arb_write_display_operation() -> impl Strategy<Value = Operation> {
        (
            any::<String>(),
            prop_oneof![
                Just(Ok(())),
                any::<String>().prop_map(|v| Err(Error::UnexpectedDisplay(v))),
                Just(Err(Error::WrongWrite(
                    OperationType::WriteNumber,
                    OperationType::WriteDisplay
                ))),
                Just(Err(Error::WrongWrite(
                    OperationType::WriteSlice,
                    OperationType::WriteDisplay
                ))),
                any::<String>().prop_map(|s| Err(Error::Custom(s))),
                (any::<io::ErrorKind>(), any::<String>())
                    .prop_map(|(kind, msg)| Err(Error::IO(kind, msg))),
            ],
        )
            .prop_filter("same string", |(v, res)| match res {
                Err(Error::UnexpectedDisplay(exp_v)) => v != exp_v,
                _ => true,
            })
            .prop_map(|(v, res)| Operation::WriteDisplay(v, res))
    }

    pub fn arb_operation() -> impl Strategy<Value = Operation> {
        prop_oneof![
            arb_write_number_operation(),
            arb_write_slice_operation(),
            arb_write_display_operation(),
        ]
    }

    impl Arbitrary for Operation {
        type Parameters = ();
        type Strategy = BoxedStrategy<Operation>;

        fn arbitrary_with(_args: Self::Parameters) -> Self::Strategy {
            arb_operation().boxed()
        }
    }
}

#[cfg(test)]
mod test {
    use hex_literal::hex;
    use proptest::prelude::any;
    use proptest::prelude::TestCaseError;
    use proptest::proptest;

    use crate::wire::ser::mock::proptest::arb_extra_write;
    use crate::wire::ser::mock::Operation;
    use crate::wire::ser::mock::OperationType;
    use crate::wire::ser::Error as _;
    use crate::wire::ser::NixWrite;

    use super::{Builder, Error};

    #[tokio::test]
    async fn write_number() {
        let mut mock = Builder::new().write_number(10).build();
        mock.write_number(10).await.unwrap();
    }

    #[tokio::test]
    async fn write_number_error() {
        let mut mock = Builder::new()
            .write_number_error(10, Error::custom("bad number"))
            .build();
        assert_eq!(
            Err(Error::custom("bad number")),
            mock.write_number(10).await
        );
    }

    #[tokio::test]
    async fn write_number_unexpected() {
        let mut mock = Builder::new().write_slice(b"").build();
        assert_eq!(
            Err(Error::unexpected_write_number(OperationType::WriteSlice)),
            mock.write_number(11).await
        );
    }

    #[tokio::test]
    async fn write_number_unexpected_number() {
        let mut mock = Builder::new().write_number(10).build();
        assert_eq!(
            Err(Error::UnexpectedNumber(11)),
            mock.write_number(11).await
        );
    }

    #[tokio::test]
    async fn extra_write_number() {
        let mut mock = Builder::new().build();
        assert_eq!(
            Err(Error::ExtraWrite(OperationType::WriteNumber)),
            mock.write_number(11).await
        );
    }

    #[tokio::test]
    async fn write_slice() {
        let mut mock = Builder::new()
            .write_slice(&[])
            .write_slice(&hex!("0000 1234 5678 9ABC DEFF"))
            .build();
        mock.write_slice(&[]).await.expect("write_slice empty");
        mock.write_slice(&hex!("0000 1234 5678 9ABC DEFF"))
            .await
            .expect("write_slice");
    }

    #[tokio::test]
    async fn write_slice_error() {
        let mut mock = Builder::new()
            .write_slice_error(&[], Error::custom("bad slice"))
            .build();
        assert_eq!(Err(Error::custom("bad slice")), mock.write_slice(&[]).await);
    }

    #[tokio::test]
    async fn write_slice_unexpected() {
        let mut mock = Builder::new().write_number(10).build();
        assert_eq!(
            Err(Error::unexpected_write_slice(OperationType::WriteNumber)),
            mock.write_slice(b"").await
        );
    }

    #[tokio::test]
    async fn write_slice_unexpected_slice() {
        let mut mock = Builder::new().write_slice(b"").build();
        assert_eq!(
            Err(Error::UnexpectedSlice(b"bad slice".to_vec())),
            mock.write_slice(b"bad slice").await
        );
    }

    #[tokio::test]
    async fn extra_write_slice() {
        let mut mock = Builder::new().build();
        assert_eq!(
            Err(Error::ExtraWrite(OperationType::WriteSlice)),
            mock.write_slice(b"extra slice").await
        );
    }

    #[tokio::test]
    async fn write_display() {
        let mut mock = Builder::new().write_display("testing").build();
        mock.write_display("testing").await.unwrap();
    }

    #[tokio::test]
    async fn write_display_error() {
        let mut mock = Builder::new()
            .write_display_error("testing", Error::custom("bad number"))
            .build();
        assert_eq!(
            Err(Error::custom("bad number")),
            mock.write_display("testing").await
        );
    }

    #[tokio::test]
    async fn write_display_unexpected() {
        let mut mock = Builder::new().write_number(10).build();
        assert_eq!(
            Err(Error::unexpected_write_display(OperationType::WriteNumber)),
            mock.write_display("").await
        );
    }

    #[tokio::test]
    async fn write_display_unexpected_display() {
        let mut mock = Builder::new().write_display("").build();
        assert_eq!(
            Err(Error::UnexpectedDisplay("bad display".to_string())),
            mock.write_display("bad display").await
        );
    }

    #[tokio::test]
    async fn extra_write_display() {
        let mut mock = Builder::new().build();
        assert_eq!(
            Err(Error::ExtraWrite(OperationType::WriteDisplay)),
            mock.write_display("extra slice").await
        );
    }

    #[test]
    #[should_panic]
    fn operations_left() {
        let _ = Builder::new().write_number(10).build();
    }

    #[test]
    fn proptest_mock() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        proptest!(|(
            operations in any::<Vec<Operation>>(),
            extra_operations in proptest::collection::vec(arb_extra_write(), 0..3)
            )| {
            rt.block_on(async {
                let mut builder = Builder::new();
                for op in operations.iter() {
                    builder.write_operation(op);
                }
                for op in extra_operations.iter() {
                    builder.write_operation(op);
                }
                let mut mock = builder.build();
                for op in operations {
                    mock.prop_assert_operation(op).await?;
                }
                for op in extra_operations {
                    mock.prop_assert_operation(op).await?;
                }
                Ok(()) as Result<(), TestCaseError>
            })?;
        });
    }
}
