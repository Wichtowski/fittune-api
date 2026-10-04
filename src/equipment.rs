use serde::{Deserialize, Serialize};

/// A specific piece of equipment. Places list the items they have and exercises list the items
/// they need, so an exercise can be done at a place when all its items are there.
/// The `equipment_items()` SQL function, last replaced by
/// `migrations/20261004000001_exercise_catalog_schema.sql`, holds the same list in the same order
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, sqlx::Type,
)]
#[sqlx(type_name = "text", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum EquipmentItem {
    // Free weights
    Barbell,
    EzBar,
    TrapBar,
    Dumbbells,
    Kettlebells,
    WeightPlates,
    // Benches and racks
    FlatBench,
    AdjustableBench,
    PreacherBench,
    BackExtensionBench,
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
    /// Any strength machine without an item of its own, e.g. hack squat or preacher curl
    StrengthMachines,
    // Cable
    CableStation,
    LatPulldown,
    SeatedRow,
    // Cardio
    Treadmill,
    RowingMachine,
    StationaryBike,
    /// Any cardio machine without an item of its own, e.g. elliptical or stair climber
    CardioMachines,
    // Accessories
    ResistanceBand,
    SuspensionTrainer,
    StabilityBall,
    BosuBall,
    MedicineBall,
    FoamRoller,
    PlyoBox,
    AbWheel,
    JumpRope,
    BattleRopes,
    ClimbingRope,
    SledgehammerTire,
}

/// Sorts into declaration order and removes duplicates, so equal sets compare equal
pub fn canonical(mut items: Vec<EquipmentItem>) -> Vec<EquipmentItem> {
    items.sort();
    items.dedup();
    items
}
