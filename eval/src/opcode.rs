//! This module implements the instruction set running on the abstract
//! machine implemented by tvix.

#[derive(Clone, Copy, Debug)]
pub struct ConstantIdx(pub usize);

#[derive(Clone, Copy, Debug)]
pub struct CodeIdx(pub usize);

#[derive(Clone, Copy, Debug)]
pub enum OpCode {
    // Push a constant onto the stack.
    OpConstant(ConstantIdx),

    // Discard a value from the stack.
    OpPop,

    // Push a literal value.
    OpNull,
    OpTrue,
    OpFalse,

    // Unary operators
    OpInvert,
    OpNegate,

    // Arithmetic binary operators
    OpAdd,
    OpSub,
    OpMul,
    OpDiv,

    // Comparison operators
    OpEqual,
    OpLess,
    OpLessOrEq,
    OpMore,
    OpMoreOrEq,

    // Logical operators & generic jumps
    OpJump(usize),
    OpJumpIfFalse(usize),

    // Attribute sets
    OpAttrs(usize),
    OpAttrPath(usize),
    OpAttrsUpdate,

    // Lists
    OpList(usize),
    OpConcat,

    // Strings
    OpInterpolate(usize),
}
