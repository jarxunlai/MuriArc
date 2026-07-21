use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{DomainError, RecordMeta, require_non_empty};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lab {
    pub id: Uuid,
    pub name: String,
    pub meta: RecordMeta,
}

impl Lab {
    pub fn new(name: impl Into<String>, now: DateTime<Utc>) -> Result<Self, DomainError> {
        let name = name.into();
        require_non_empty("lab.name", &name)?;
        Ok(Self {
            id: Uuid::new_v4(),
            name,
            meta: RecordMeta::new(now),
        })
    }

    pub fn rename(
        &mut self,
        name: impl Into<String>,
        now: DateTime<Utc>,
    ) -> Result<(), DomainError> {
        let name = name.into();
        require_non_empty("lab.name", &name)?;
        self.name = name.trim().to_owned();
        self.meta.touch(now);
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserStatus {
    Active,
    Suspended,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    pub id: Uuid,
    pub lab_id: Uuid,
    pub email: String,
    pub display_name: String,
    pub status: UserStatus,
    pub meta: RecordMeta,
}

impl User {
    pub fn new(
        lab_id: Uuid,
        email: impl Into<String>,
        display_name: impl Into<String>,
        now: DateTime<Utc>,
    ) -> Result<Self, DomainError> {
        let email = email.into();
        let display_name = display_name.into();
        require_non_empty("user.email", &email)?;
        require_non_empty("user.display_name", &display_name)?;
        let email = email.trim().to_ascii_lowercase();
        let display_name = display_name.trim().to_owned();
        validate_email(&email)?;
        validate_display_name(&display_name)?;
        Ok(Self {
            id: Uuid::new_v4(),
            lab_id,
            email,
            display_name,
            status: UserStatus::Active,
            meta: RecordMeta::new(now),
        })
    }

    pub fn rename(
        &mut self,
        display_name: impl Into<String>,
        now: DateTime<Utc>,
    ) -> Result<(), DomainError> {
        let display_name = display_name.into();
        require_non_empty("user.display_name", &display_name)?;
        let display_name = display_name.trim().to_owned();
        validate_display_name(&display_name)?;
        self.display_name = display_name;
        self.meta.touch(now);
        Ok(())
    }

    pub fn suspend(&mut self, now: DateTime<Utc>) {
        if self.status != UserStatus::Suspended {
            self.status = UserStatus::Suspended;
            self.meta.touch(now);
        }
    }

    pub fn reactivate(&mut self, now: DateTime<Utc>) {
        if self.status != UserStatus::Active {
            self.status = UserStatus::Active;
            self.meta.touch(now);
        }
    }
}

fn validate_email(email: &str) -> Result<(), DomainError> {
    if email.len() > 320
        || !email.contains('@')
        || email.chars().any(char::is_control)
        || email.chars().any(char::is_whitespace)
    {
        Err(DomainError::InvalidUserEmail)
    } else {
        Ok(())
    }
}

fn validate_display_name(display_name: &str) -> Result<(), DomainError> {
    if display_name.is_empty()
        || display_name.chars().count() > 200
        || display_name.chars().any(char::is_control)
    {
        Err(DomainError::InvalidUserDisplayName)
    } else {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectStatus {
    Active,
    Archived,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub id: Uuid,
    pub lab_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub status: ProjectStatus,
    pub meta: RecordMeta,
}

impl Project {
    pub fn new(
        lab_id: Uuid,
        name: impl Into<String>,
        now: DateTime<Utc>,
    ) -> Result<Self, DomainError> {
        let name = name.into();
        require_non_empty("project.name", &name)?;
        Ok(Self {
            id: Uuid::new_v4(),
            lab_id,
            name,
            description: None,
            status: ProjectStatus::Active,
            meta: RecordMeta::new(now),
        })
    }
}
