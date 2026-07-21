#![forbid(unsafe_code)]

mod animal;
mod breeding;
mod cage;
mod experiment;
mod genetics;
mod observation;
mod organization;
mod project_animal;
mod records;
mod validation;

pub use animal::{
    CreateAnimalCommand, CreateAnimalIdentifierScope, MAX_ANIMAL_DISPLAY_ID_CHARS,
    MAX_ANIMAL_STRAIN_CHARS, create_animal,
};
pub use breeding::*;
pub use cage::{
    CreateCageCommand, MAX_CAGE_DISPLAY_ID_CHARS, MAX_CAGE_LOCATION_CHARS, MAX_CAGE_SECTION_CHARS,
    MAX_TRANSFER_NOTES_BYTES, TransferAnimalsCommand, create_cage, transfer_animals,
};
pub use experiment::*;
pub use genetics::*;
use muriarc_core::{DomainError, StoreError};
pub use observation::*;
pub use organization::*;
pub use project_animal::*;
pub use records::*;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ApplicationError {
    #[error(transparent)]
    Domain(#[from] DomainError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("{field} must not exceed {max} characters")]
    TooLong { field: &'static str, max: usize },
    #[error("{field} must not exceed {max} bytes")]
    TooManyBytes { field: &'static str, max: usize },
    #[error("validation failed: {0}")]
    Validation(String),
}

pub type ApplicationResult<T> = Result<T, ApplicationError>;
