//! Daily calorie and macro targets. Pure maths, so the rules are easy to read and test

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "text", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum Sex {
    Male,
    Female,
}

/// Daily life without exercise; exercise is added per day from FitTune
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "text", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum Activity {
    Sedentary,
    Light,
    Moderate,
    Active,
    VeryActive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "text", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum Goal {
    Lose,
    Maintain,
    Gain,
}

/// Energy target before the training bonus never goes below this
pub const MIN_KCAL: f64 = 1200.0;
/// Energy in one kilogram of body weight change
const KCAL_PER_KG: f64 = 7700.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Body {
    pub sex: Sex,
    pub age_years: u32,
    pub height_cm: f64,
    pub weight_kg: f64,
    pub activity: Activity,
    pub goal: Goal,
    pub pace_kg_per_week: f64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Overrides {
    pub energy_kcal: Option<f64>,
    pub protein_g: Option<f64>,
    pub fat_g: Option<f64>,
    pub carbs_g: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Targets {
    pub energy_kcal: f64,
    pub protein_g: f64,
    pub fat_g: f64,
    pub carbs_g: f64,
    /// Part of `energy_kcal` that comes from the day's training
    pub training_kcal: f64,
}

/// Mifflin-St Jeor resting energy
pub fn bmr(body: &Body) -> f64 {
    let sex = match body.sex {
        Sex::Male => 5.0,
        Sex::Female => -161.0,
    };
    10.0 * body.weight_kg + 6.25 * body.height_cm - 5.0 * f64::from(body.age_years) + sex
}

pub fn activity_factor(activity: Activity) -> f64 {
    match activity {
        Activity::Sedentary => 1.2,
        Activity::Light => 1.375,
        Activity::Moderate => 1.55,
        Activity::Active => 1.725,
        Activity::VeryActive => 1.9,
    }
}

/// Energy for the goal before training: daily life plus or minus the weekly pace, never below
/// [`MIN_KCAL`]
fn base_energy(body: &Body) -> f64 {
    let pace = body.pace_kg_per_week * KCAL_PER_KG / 7.0;
    let adjustment = match body.goal {
        Goal::Lose => -pace,
        Goal::Maintain => 0.0,
        Goal::Gain => pace,
    };
    (bmr(body) * activity_factor(body.activity) + adjustment).max(MIN_KCAL)
}

/// Targets for one day. `body` is `None` when the profile, birthday or weight is missing; then
/// only overrides can produce targets, and without an energy target there are none
pub fn daily(body: Option<&Body>, overrides: Overrides, training_kcal: f64) -> Option<Targets> {
    let energy = overrides.energy_kcal.or_else(|| body.map(base_energy))?;
    let weight = body.map(|b| b.weight_kg);

    // Protein per kilogram, more while losing to keep muscle; a fifth of energy without a weight
    let protein = overrides.protein_g.unwrap_or_else(|| match body {
        Some(b) if b.goal == Goal::Lose => 2.0 * b.weight_kg,
        Some(b) => 1.8 * b.weight_kg,
        None => energy * 0.2 / 4.0,
    });
    let fat = overrides.fat_g.unwrap_or_else(|| {
        let quarter = energy * 0.25 / 9.0;
        weight.map_or(quarter, |kg| quarter.max(0.6 * kg))
    });
    let carbs = overrides
        .carbs_g
        .unwrap_or_else(|| ((energy - protein * 4.0 - fat * 9.0) / 4.0).max(0.0));

    // Training is fuelled by carbohydrate, so the bonus lands there
    Some(Targets {
        energy_kcal: energy + training_kcal,
        protein_g: protein,
        fat_g: fat,
        carbs_g: carbs + training_kcal / 4.0,
        training_kcal,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body() -> Body {
        Body {
            sex: Sex::Male,
            age_years: 30,
            height_cm: 180.0,
            weight_kg: 80.0,
            activity: Activity::Moderate,
            goal: Goal::Maintain,
            pace_kg_per_week: 0.0,
        }
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 0.01
    }

    #[test]
    fn bmr_follows_mifflin_st_jeor() {
        // 10 * 80 + 6.25 * 180 - 5 * 30 + 5
        assert!(close(bmr(&body()), 1780.0));
        let female = Body {
            sex: Sex::Female,
            ..body()
        };
        assert!(close(bmr(&female), 1614.0));
    }

    #[test]
    fn maintenance_is_bmr_times_daily_life() {
        let t = daily(Some(&body()), Overrides::default(), 0.0).expect("targets");
        assert!(close(t.energy_kcal, 1780.0 * 1.55));
        assert!(close(t.training_kcal, 0.0));
    }

    #[test]
    fn losing_takes_the_weekly_pace_off_every_day() {
        let losing = Body {
            goal: Goal::Lose,
            pace_kg_per_week: 0.5,
            ..body()
        };
        let t = daily(Some(&losing), Overrides::default(), 0.0).expect("targets");
        assert!(close(t.energy_kcal, 1780.0 * 1.55 - 550.0));
        // Losing keeps more protein
        assert!(close(t.protein_g, 160.0));
    }

    #[test]
    fn gaining_adds_the_weekly_pace() {
        let gaining = Body {
            goal: Goal::Gain,
            pace_kg_per_week: 0.25,
            ..body()
        };
        let t = daily(Some(&gaining), Overrides::default(), 0.0).expect("targets");
        assert!(close(t.energy_kcal, 1780.0 * 1.55 + 275.0));
        assert!(close(t.protein_g, 144.0));
    }

    #[test]
    fn never_goes_below_the_floor_before_training() {
        let small = Body {
            sex: Sex::Female,
            height_cm: 150.0,
            weight_kg: 45.0,
            activity: Activity::Sedentary,
            goal: Goal::Lose,
            pace_kg_per_week: 1.0,
            ..body()
        };
        let t = daily(Some(&small), Overrides::default(), 300.0).expect("targets");
        assert!(close(t.energy_kcal, MIN_KCAL + 300.0));
    }

    #[test]
    fn macros_split_protein_fat_then_carbs() {
        let t = daily(Some(&body()), Overrides::default(), 0.0).expect("targets");
        let kcal = 1780.0 * 1.55;
        assert!(close(t.protein_g, 144.0));
        assert!(close(t.fat_g, kcal * 0.25 / 9.0));
        assert!(close(t.carbs_g, (kcal - 144.0 * 4.0 - t.fat_g * 9.0) / 4.0));
    }

    #[test]
    fn fat_never_drops_below_its_minimum_per_kilogram() {
        let heavy = Body {
            weight_kg: 150.0,
            activity: Activity::Sedentary,
            goal: Goal::Lose,
            pace_kg_per_week: 1.0,
            ..body()
        };
        let t = daily(Some(&heavy), Overrides::default(), 0.0).expect("targets");
        assert!(t.fat_g >= 0.6 * 150.0 - 0.01);
        assert!(t.carbs_g >= 0.0);
    }

    #[test]
    fn training_goes_to_calories_and_carbs() {
        let rest = daily(Some(&body()), Overrides::default(), 0.0).expect("targets");
        let trained = daily(Some(&body()), Overrides::default(), 400.0).expect("targets");
        assert!(close(trained.energy_kcal, rest.energy_kcal + 400.0));
        assert!(close(trained.carbs_g, rest.carbs_g + 100.0));
        assert!(close(trained.protein_g, rest.protein_g));
        assert!(close(trained.training_kcal, 400.0));
    }

    #[test]
    fn an_energy_override_recomputes_macros_and_still_gets_training() {
        let overrides = Overrides {
            energy_kcal: Some(2000.0),
            ..Overrides::default()
        };
        let t = daily(Some(&body()), overrides, 200.0).expect("targets");
        assert!(close(t.energy_kcal, 2200.0));
        assert!(close(t.fat_g, 2000.0 * 0.25 / 9.0));
        assert!(close(
            t.carbs_g,
            (2000.0 - 144.0 * 4.0 - t.fat_g * 9.0) / 4.0 + 50.0
        ));
    }

    #[test]
    fn macro_overrides_win() {
        let overrides = Overrides {
            protein_g: Some(200.0),
            fat_g: Some(70.0),
            carbs_g: Some(150.0),
            ..Overrides::default()
        };
        let t = daily(Some(&body()), overrides, 100.0).expect("targets");
        assert!(close(t.protein_g, 200.0));
        assert!(close(t.fat_g, 70.0));
        // An explicit carb target still grows with training
        assert!(close(t.carbs_g, 175.0));
    }

    #[test]
    fn without_body_data_only_overrides_count() {
        assert!(daily(None, Overrides::default(), 300.0).is_none());
        let overrides = Overrides {
            energy_kcal: Some(2100.0),
            protein_g: Some(150.0),
            ..Overrides::default()
        };
        let t = daily(None, overrides, 300.0).expect("targets");
        assert!(close(t.energy_kcal, 2400.0));
        assert!(close(t.protein_g, 150.0));
        // Fat falls back to a quarter of the energy target without a weight
        assert!(close(t.fat_g, 2100.0 * 0.25 / 9.0));
        assert!(close(
            t.carbs_g,
            (2100.0 - 600.0 - t.fat_g * 9.0) / 4.0 + 75.0
        ));
    }
}
