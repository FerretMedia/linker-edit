//! The agents' study tools: measuring example videos and lining a cut up with its raw footage. The port of
//! server/study. [`study::StudyService`] is the service the app holds.

pub mod compare;
pub mod measure;
#[allow(clippy::module_inception)]
pub mod study;

pub use study::{StudyOptions, StudyOutput, StudyService, is_study_tool, study_tools};
