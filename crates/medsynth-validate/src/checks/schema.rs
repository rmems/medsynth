use crate::*;

/// `schema` — JSON Schema validation plus deserialize (of the raw record
/// text, so duplicate fields fail) + cross-field `validate()`.
pub(super) fn schema_check(
    record: &RecordFile,
    validators: &Validators,
    findings: &mut Vec<Finding>,
) {
    let mut push = |detail: String| {
        findings.push(Finding {
            path: record.path.clone(),
            line: record.line,
            record_id: record.record_id(),
            check: Check::Schema,
            detail,
        });
    };
    let Some(validator) = validators.by_format.get(&record.format) else {
        push(format!("no schema for format {}", record.format.slug()));
        return;
    };
    let schema_errors: Vec<String> = validator
        .iter_errors(&record.value)
        .map(|e| format!("{e} at {}", e.instance_path()))
        .collect();
    if !schema_errors.is_empty() {
        for detail in schema_errors {
            push(detail);
        }
        return;
    }
    // Deserialization failure names the serde path; a validate() failure
    // lists the cross-field rules broken.
    match record.format {
        Format::Mcq => cross_detail::<McqRecord>(record, findings),
        Format::CaseVignette => cross_detail::<CaseVignetteRecord>(record, findings),
        Format::TutoringExample => cross_detail::<TutoringExampleRecord>(record, findings),
    }
}

fn cross_detail<T>(record: &RecordFile, findings: &mut Vec<Finding>)
where
    T: serde::de::DeserializeOwned + CrossValidate,
{
    let mut push = |detail: String| {
        findings.push(Finding {
            path: record.path.clone(),
            line: record.line,
            record_id: record.record_id(),
            check: Check::Schema,
            detail,
        });
    };
    match serde_json::from_str::<T>(&record.raw) {
        Err(e) => push(format!("record does not deserialize: {e}")),
        Ok(r) => {
            if let Err(errors) = r.validate() {
                for e in errors {
                    push(format!("cross-field rule: {e}"));
                }
            }
        }
    }
}

/// Lets `schema_check` call each record type's `validate` generically.
pub trait CrossValidate {
    /// Cross-field `validate` from `medsynth-schema`.
    fn validate(&self) -> Result<(), Vec<ValidationError>>;
}
impl CrossValidate for McqRecord {
    fn validate(&self) -> Result<(), Vec<ValidationError>> {
        McqRecord::validate(self)
    }
}
impl CrossValidate for CaseVignetteRecord {
    fn validate(&self) -> Result<(), Vec<ValidationError>> {
        CaseVignetteRecord::validate(self)
    }
}
impl CrossValidate for TutoringExampleRecord {
    fn validate(&self) -> Result<(), Vec<ValidationError>> {
        TutoringExampleRecord::validate(self)
    }
}
