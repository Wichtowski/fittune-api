use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    equipment::{self, EquipmentItem},
    error::{ApiResult, FieldErrors},
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
    pub equipment: Vec<EquipmentItem>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlaceRequest {
    pub name: String,
    pub kind: PlaceKind,
    pub equipment: Vec<EquipmentItem>,
}

impl PlaceRequest {
    pub fn validate(mut self) -> ApiResult<Self> {
        let mut errors = FieldErrors::default();
        self.name = validate::required_text(&mut errors, "name", &self.name, 1, 80);
        // Canonical order, so reordering the same equipment does not create a new version
        self.equipment = equipment::canonical(self.equipment);
        errors.into_result()?;
        Ok(self)
    }
}
