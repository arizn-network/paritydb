//! Decision
//!
//! Our database supports two types of operations. Inserts and deletes.
//! This module is responsible for comparing existing records with new operations
//! and making decisions based on the result of this comparison. The decision is
//! later used to create idempotent database operation.

use std::cmp;
use key::Key;
use record::Record;
use space::Space;
use transaction::Operation;

/// Decision made after comparing existing record and new operation.
#[derive(Debug)]
pub enum Decision<'o, 'db> {
	/// Returned when operation is an insert operation and the space is empty
	InsertOperationIntoEmptySpace {
		key: &'o [u8],
		value: &'o [u8],
		offset: usize,
		space_len: usize,
	},
	/// Returned when operation is an insert operation and it's key is lower
	/// then existing record's key
	///
	/// Operation should be marked as inserted.
	/// Operations iterator should be moved to the next value.
	/// Spaces iterator offset should be moved to the next operation location.
	InsertOperationBeforeOccupiedSpace {
		key: &'o [u8],
		value: &'o [u8],
		offset: usize,
	},
	/// Returns when operation is an insert operation and it's key is equal
	/// to existing record's key.
	OverwriteOperation {
		key: &'o [u8],
		value: &'o [u8],
		offset: usize,
		old_len: usize,
	},
	/// Returns when operation is a delete operation and it's key is equal
	/// to existing record's key.
	///
	/// The record should be marked as deleted.
	DeleteOperation {
		offset: usize,
		len: usize,
	},
	/// Space is occupied and existing record's key is greater then operation's key.
	/// No decision could be made.
	///
	/// If it's occupied space, it should be appended to current idempotent operation.
	SeekSpace,
	/// Returned only on delete, when deleted value is not found in the database.
	///
	/// Operations iterator should be moved to the next value.
	/// Spaces iterator offset should be moved to next operation location.
	IgnoreOperation,
	ConsumeEmptySpace {
		len: usize,
	},
	ShiftOccupiedSpace {
		data: &'db [u8],
	},
	FinishDeletedSpace
}

/// Compares occupied space data and operation key.
#[inline]
fn compare_space_and_operation(space: &[u8], key: &[u8], field_body_size: usize) -> cmp::Ordering {
	Record::extract_key(space, field_body_size, key.len()).partial_cmp(&key).unwrap()
}

#[inline]
pub fn is_min_offset_for_insert(offset: usize, shift: isize, key: &[u8], prefix_bits: u8, field_body_size: usize) -> bool {
	let offset = offset - (-shift) as usize;
	let prefixed_key = Key::new(key, prefix_bits);
	let min_offset = prefixed_key.offset(field_body_size);
	min_offset <= offset
}

#[inline]
pub fn is_min_offset_for_shift(_offset: usize, _shift: isize, _data: &[u8], _prefix_bits: u8, _field_body_size: usize) -> bool {
	// TODO: get desired offset from data and compare it with offset - shift
	false
}

#[derive(Debug, PartialEq, Clone, Copy)]
enum Shift {
	None,
	Forward,
	Backward,
}

impl From<isize> for Shift {
	fn from(shift: isize) -> Self {
		match shift.cmp(&0) {
			cmp::Ordering::Equal => Shift::None,
			cmp::Ordering::Greater => Shift::Forward,
			cmp::Ordering::Less => Shift::Backward,
		}
	}
}

pub fn decision<'o, 'db>(operation: Operation<'o>, space: Space<'db>, shift: isize, field_body_size: usize, prefix_bits: u8) -> Decision<'o, 'db> {
	let tip = shift.into();
	match (operation, space, tip) {
		(Operation::Insert(key, value), Space::Empty(space), Shift::None) => Decision::InsertOperationIntoEmptySpace {
			key,
			value,
			offset: space.offset,
			space_len: space.len,
		},
		(Operation::Insert(key, value), Space::Empty(space), Shift::Backward) => if is_min_offset_for_insert(space.offset, shift, key, prefix_bits, field_body_size) {
			Decision::InsertOperationIntoEmptySpace {
				key,
				value,
				offset: space.offset,
				space_len: space.len,
			}
		} else {
			Decision::FinishDeletedSpace
		},
		(Operation::Insert(_, _), Space::Empty(space), Shift::Forward) => Decision::ConsumeEmptySpace {
			len: space.len,
		},
		(Operation::Insert(key, value), Space::Occupied(space), _) => {
			match (compare_space_and_operation(space.data, key, field_body_size), tip) {
				(cmp::Ordering::Less, Shift::None) => Decision::SeekSpace,
				(cmp::Ordering::Less, Shift::Backward) => if is_min_offset_for_shift(space.offset, shift, space.data, prefix_bits, field_body_size) {
					Decision::ShiftOccupiedSpace {
						data: space.data,
					}
				} else {
					Decision::FinishDeletedSpace
				},
				(cmp::Ordering::Less, Shift::Forward) => Decision::ShiftOccupiedSpace {
					data: space.data,
				},
				(cmp::Ordering::Equal, _) => Decision::OverwriteOperation {
					key,
					value,
					offset: space.offset,
					old_len: space.data.len()
				},
				(cmp::Ordering::Greater, Shift::Backward) => if is_min_offset_for_insert(space.offset, shift, key, prefix_bits, field_body_size) {
					Decision::InsertOperationBeforeOccupiedSpace {
						key,
						value,
						offset: space.offset,
					}
				} else {
					Decision::FinishDeletedSpace
				},
				(cmp::Ordering::Greater, Shift::None) | (cmp::Ordering::Greater , Shift::Forward) => Decision::InsertOperationBeforeOccupiedSpace {
					key,
					value,
					offset: space.offset,
				}
			}
		},
		// record does not exist
		(Operation::Delete(_), Space::Empty(_), Shift::None) => Decision::IgnoreOperation,
		(Operation::Delete(_), Space::Empty(space), _) => Decision::ConsumeEmptySpace {
			len: space.len,
		},
		(Operation::Delete(key), Space::Occupied(space), _) => {
			match (compare_space_and_operation(space.data, key, field_body_size), tip) {
				(cmp::Ordering::Less, Shift::None) => Decision::SeekSpace,
				(cmp::Ordering::Less, Shift::Backward) => if is_min_offset_for_shift(space.offset, shift, space.data, prefix_bits, field_body_size) {
					Decision::ShiftOccupiedSpace {
						data: space.data,
					}
				} else {
					Decision::FinishDeletedSpace
				},
				(cmp::Ordering::Less, Shift::Forward) => Decision::ShiftOccupiedSpace {
					data: space.data,
				},
				(cmp::Ordering::Equal, _) => Decision::DeleteOperation {
					offset: space.offset,
					len: space.data.len(),
				},
				// record does not exist
				(cmp::Ordering::Greater, _) => Decision::IgnoreOperation,
			}
		},
	}
}