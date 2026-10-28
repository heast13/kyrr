//! Voice memos: a key (F8 by default) or a button starts a recording into
//! Documents/kyrr, the same key ends it. kyrr names it after the first
//! words said in it, lists the folder, plays a memo back and, on a click,
//! writes down what was said, all on the computer.

pub mod compare;
pub mod library;
pub mod opusfile;
pub mod player;
pub mod recorder;
pub mod text;
pub mod title;
