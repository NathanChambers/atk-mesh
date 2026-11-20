use crate::objects::{MeshObject, ObjectStage};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;

const AREAS: [&str; 4] = ["form", "interfaces", "finish", "technical_fit"];
const REFINED_AREAS: [&str; 3] = ["form", "interfaces", "required_features"];

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Check {
    id: String,
    area: String,
    criterion: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    result: Option<Assessment>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Assessment {
    outcome: Outcome,
    reason: String,
    #[serde(default)]
    evidence: Vec<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    workspace_evidence: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Outcome {
    Pass,
    NeedsWork,
    NotApplicable,
}

fn valid_text(value: &str, maximum: usize) -> bool {
    !value.trim().is_empty() && value.len() <= maximum
}

fn valid_plan(checks: &[Check]) -> bool {
    let mut ids = BTreeSet::new();
    checks.len() <= 32
        && AREAS
            .iter()
            .all(|area| checks.iter().any(|check| &check.area == area))
        && checks.iter().all(|check| {
            (AREAS.contains(&check.area.as_str()) || check.area == "required_features")
                && valid_text(&check.id, 64)
                && valid_text(&check.criterion, 1024)
                && ids.insert(&check.id)
        })
}

fn check_id(request: &Value) -> Result<&str, String> {
    request["check"]
        .as_str()
        .ok_or_else(|| "check ID is required".into())
}

fn assessment(request: &Value) -> Result<Assessment, String> {
    let mut result: Assessment = serde_json::from_value(json!({
        "outcome":request["outcome"],
        "reason":request["reason"],
        "evidence":request.get("evidence").cloned().unwrap_or_else(|| json!([])),
        "workspace_evidence":request["workspace_evidence"] == true,
    }))
    .map_err(|_| "provide outcome: pass, needs_work or not_applicable, a reason and optional evidence IDs")?;
    result.reason = result.reason.trim().into();
    if !valid_text(&result.reason, 1024) || result.evidence.len() > 16 {
        return Err("provide a 1 to 1024 byte reason and up to 16 evidence IDs".into());
    }
    if result.outcome == Outcome::Pass && result.evidence.is_empty() {
        return Err("pass requires current evidence".into());
    }
    if result.outcome == Outcome::NotApplicable && !result.evidence.is_empty() {
        return Err("not_applicable requires a reason only".into());
    }
    Ok(result)
}

fn demote(object: &mut MeshObject) {
    object.stage = match object.stage {
        ObjectStage::Refined => ObjectStage::Blockout,
        ObjectStage::ProductionReady => ObjectStage::Detailed,
        _ => return,
    };
    object.stage_note = None;
}

pub(super) fn reopen(object: &mut MeshObject) {
    demote(object);
    for check in &mut object.review {
        check.result = None;
    }
}

pub(super) fn has_results(object: &MeshObject) -> bool {
    object.review.iter().any(|check| check.result.is_some())
}

pub(super) fn has_workspace_evidence(object: &MeshObject) -> bool {
    object.review.iter().any(|check| {
        check
            .result
            .as_ref()
            .is_some_and(|result| result.workspace_evidence)
    })
}

pub(super) fn reopen_workspace_checks(object: &mut MeshObject) {
    if object.stage == ObjectStage::Refined {
        demote(object);
    }
    if has_workspace_evidence(object) {
        demote(object);
        for check in &mut object.review {
            if check
                .result
                .as_ref()
                .is_some_and(|result| result.workspace_evidence)
            {
                check.result = None;
            }
        }
    }
}

pub(super) fn refined_blockers(checks: &[Check]) -> Vec<Value> {
    let mut blockers = Vec::new();
    for area in REFINED_AREAS {
        let mut area_checks = checks.iter().filter(|check| check.area == area).peekable();
        if area_checks.peek().is_none() {
            blockers.push(json!({"area":area,"status":"missing_check"}));
        }
        for check in area_checks {
            let outcome = check.result.as_ref().map(|result| result.outcome);
            let resolved = outcome == Some(Outcome::Pass)
                || (area == "interfaces" && outcome == Some(Outcome::NotApplicable));
            if !resolved {
                blockers.push(json!({
                    "area":area,"check":check.id,"criterion":check.criterion,
                    "status":outcome.map_or(json!("pending"), |outcome| json!(outcome)),
                    "reason":check.result.as_ref().map(|result| &result.reason)
                }));
            }
        }
    }
    blockers
}

pub(super) fn approved_plan(checks: &[Check]) -> bool {
    valid_plan(checks)
        && checks.iter().any(|check| {
            check
                .result
                .as_ref()
                .is_some_and(|assessment| assessment.outcome == Outcome::Pass)
        })
        && checks.iter().all(|check| {
            check.result.as_ref().is_some_and(|assessment| {
                valid_text(&assessment.reason, 1024)
                    && match assessment.outcome {
                        Outcome::Pass => !assessment.evidence.is_empty(),
                        Outcome::NotApplicable => true,
                        Outcome::NeedsWork => false,
                    }
            })
        })
}

pub(super) fn edit(
    object: &mut MeshObject,
    command: &str,
    request: &Value,
) -> Result<Value, String> {
    if command == "object_review_plan" {
        let checks: Vec<Check> = serde_json::from_value(request["checks"].clone())
            .map_err(|_| "checks must contain id, area and criterion")?;
        if !valid_plan(&checks) || checks.iter().any(|check| check.result.is_some()) {
            return Err("define 4 to 32 checks with unique IDs covering form, interfaces, finish and technical_fit".into());
        }
        object.review = checks;
    } else {
        let id = check_id(request)?;
        let result = assessment(request)?;
        let check = object
            .review
            .iter_mut()
            .find(|check| check.id == id)
            .ok_or("unknown check; define it with object_review_plan")?;
        check.result = Some(result);
    }
    demote(object);
    Ok(json!({"object":request["object"],"checks":object.review,"stage":object.stage}))
}

pub(super) fn verify(
    object: &MeshObject,
    request: &Value,
    mut evidence: impl FnMut(&str) -> Result<(bool, bool), String>,
) -> Result<bool, String> {
    let mut verify_evidence = |check: &Check, result: &Assessment| {
        let mut capture = false;
        let mut workspace = false;
        for id in &result.evidence {
            let (is_capture, is_workspace) = evidence(id)?;
            capture |= is_capture;
            workspace |= is_workspace;
        }
        let visual_pass = result.outcome == Outcome::Pass && check.area != "technical_fit";
        if visual_pass && !capture {
            return Err(format!("check '{}' requires a current capture", check.id));
        }
        Ok(workspace)
    };
    if request["command"] == "object_review_check" {
        let id = check_id(request)?;
        let check = object
            .review
            .iter()
            .find(|check| check.id == id)
            .ok_or("unknown check; define it with object_review_plan")?;
        return verify_evidence(check, &assessment(request)?);
    }
    let refined = request["stage"] == "refined";
    if !refined && !approved_plan(&object.review) {
        return Err("production_ready requires all checks resolved and at least one pass; use query_object_review".into());
    }
    let checks = object.review.iter().filter(|check| !refined || REFINED_AREAS.contains(&check.area.as_str()));
    for check in checks {
        verify_evidence(check, check.result.as_ref().unwrap())?;
    }
    Ok(false)
}
