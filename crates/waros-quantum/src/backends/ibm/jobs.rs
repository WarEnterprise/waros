use std::time::Duration;

use crate::backends::JobStatus;

use super::types::JobInfoResponse;

pub(crate) fn map_job_status(response: &JobInfoResponse) -> JobStatus {
    let state = response.state.status.as_str();
    match state {
        "Queued" => JobStatus::Queued { position: None },
        "Running" => JobStatus::Running,
        "Completed" => JobStatus::Completed,
        "Cancelled" | "Cancelled - Ran too long" => JobStatus::Cancelled,
        "Failed" => JobStatus::Failed {
            error: failure_reason(response),
        },
        _ => JobStatus::Failed {
            error: format!("Unknown IBM job state '{state}'"),
        },
    }
}

pub(crate) fn next_poll_interval(current: Duration) -> Duration {
    (current * 2).min(Duration::from_secs(30))
}

pub(crate) fn failure_reason(response: &JobInfoResponse) -> String {
    let mut parts = Vec::new();
    if let Some(reason) = response.state.reason.as_deref() {
        if !reason.trim().is_empty() {
            parts.push(reason.trim().to_string());
        }
    }
    if let Some(solution) = response.state.reason_solution.as_deref() {
        if !solution.trim().is_empty() {
            parts.push(solution.trim().to_string());
        }
    }
    if parts.is_empty() {
        format!("IBM job {} failed", response.id)
    } else {
        parts.join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backends::ibm::types::JobState;
    use crate::backends::JobStatus;

    fn response(status: &str, reason: Option<&str>) -> JobInfoResponse {
        JobInfoResponse {
            id: "job-1".into(),
            backend: Some("ibm_brisbane".into()),
            status: status.into(),
            state: JobState {
                status: status.into(),
                reason: reason.map(str::to_string),
                reason_code: None,
                reason_solution: None,
            },
            created: "2026-03-23T00:00:00Z".into(),
            estimated_running_time_seconds: None,
        }
    }

    #[test]
    fn maps_completed_status() {
        assert_eq!(
            map_job_status(&response("Completed", None)),
            JobStatus::Completed
        );
    }

    #[test]
    fn maps_queued_status_without_position() {
        assert_eq!(
            map_job_status(&response("Queued", None)),
            JobStatus::Queued { position: None }
        );
    }

    #[test]
    fn carries_failure_reason() {
        let status = map_job_status(&response("Failed", Some("Backend calibration changed")));
        assert_eq!(
            status,
            JobStatus::Failed {
                error: "Backend calibration changed".into()
            }
        );
    }
}
