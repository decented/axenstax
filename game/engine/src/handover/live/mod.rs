//! Live world-handover integration: eligibility, the source capture, the
//! direct control session between the source host and ONE selected joiner,
//! and the recipient's inactive import, latched listener and Ready.
pub mod authority;
pub mod bearer;
pub mod capture;
pub mod eligibility;
pub mod inert;
pub mod listener;
pub mod promote;
pub mod recipient;
pub mod resumer;
pub mod source;
pub mod wire;
pub mod world_lock;
pub mod copy;
