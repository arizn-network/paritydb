mod error;
mod header;
pub mod iterator;
mod offset;
mod size;
pub mod view;

pub use self::error::{Error, ErrorKind};
pub use self::header::{Header, HEADER_SIZE};
pub use self::size::{FieldSize, FieldOffsetIterator};

#[inline]
pub fn field_size(field_body_size: usize) -> usize {
	field_body_size + HEADER_SIZE
}
