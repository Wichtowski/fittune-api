use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    error::{ApiResult, FieldErrors},
    exercises::model::Equipment,
    validate,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "text", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum PlaceKind {
    Home,
    Gym,
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Place {
    pub id: Uuid,
    pub version_id: Uuid,
    pub name: String,
    pub kind: PlaceKind,
    pub equipment: Vec<Equipment>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlaceRequest {
    pub name: String,
    pub kind: PlaceKind,
    pub equipment: Vec<Equipment>,
}

impl PlaceRequest {
    pub fn validate(mut self) -> ApiResult<Self> {
        let mut errors = FieldErrors::default();
        self.name = validate::required_text(&mut errors, "name", &self.name, 1, 80);
        errors.ensure(
            self.equipment.len() <= 8,
            "equipment",
            "Choose at most 8 equipment types",
        );
        errors.ensure(
            !self.equipment.contains(&Equipment::None),
            "equipment",
            "Bodyweight is always available; use an empty list for no equipment",
        );
        // Canonical order, so reordering the same equipment does not create a new version
        self.equipment.sort();
        self.equipment.dedup();
        errors.into_result()?;
        Ok(self)
    }
}
