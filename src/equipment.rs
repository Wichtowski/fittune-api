use serde::{Deserialize, Serialize};

/// A specific piece of equipment. Places list the items they have and exercises list the items
/// they need, so an exercise can be done at a place when all its items are there.
/// `migrations/20260928000001_equipment_items.sql` holds the same list in the same order
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, sqlx::Type,
)]
#[sqlx(type_name = "text", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum EquipmentItem {
    // Free weights
    Barbell,
    EzBar,
    Dumbbells,
    Kettlebells,
    // Benches and racks
    FlatBench,
    AdjustableBench,
    SquatRack,
    PullUpBar,
    DipStation,
    // Machines
    LegPress,
    LegExtension,
    LegCurl,
    CalfRaiseMachine,
    SmithMachine,
    ChestPressMachine,
    PecDeck,
    ShoulderPressMachine,
    AssistedPullUpMachine,
    // Cable
    CableStation,
    LatPulldown,
    SeatedRow,
    // Cardio
    Treadmill,
    RowingMachine,
    StationaryBike,
    // Accessories
    ResistanceBand,
    AbWheel,
    JumpRope,
}

/// Sorts into declaration order and removes duplicates, so equal sets compare equal
pub fn canonical(mut items: Vec<EquipmentItem>) -> Vec<EquipmentItem> {
    items.sort();
    items.dedup();
    items
}
