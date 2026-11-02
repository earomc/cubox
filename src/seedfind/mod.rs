//! Finding seeds with special characteristics: a search is a list of conditions in plain text, see
//! `spec`, which `engine` checks against a seed from the cheapest to the most expensive. `run` goes
//! through many seeds on threads.

pub mod engine;
pub mod run;
pub mod spec;
