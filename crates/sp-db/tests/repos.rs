use sp_core::{AnalysisStatus, Ecosystem};
use sp_db::repos::*;

// These tests require a running Postgres. Set DATABASE_URL env var.
// Uses sqlx::test which creates a temporary database per test.

#[sqlx::test(migrator = "sp_db::MIGRATOR")]
async fn test_package_find_or_create(pool: sqlx::PgPool) {
    let pkg = PackageRepo::find_or_create(&pool, Ecosystem::PyPI, "Requests", "requests")
        .await
        .unwrap();
    assert_eq!(pkg.ecosystem, Ecosystem::PyPI);
    assert_eq!(pkg.name, "Requests");
    assert_eq!(pkg.normalized_name, "requests");

    // Idempotent: calling again returns same package.
    let pkg2 = PackageRepo::find_or_create(&pool, Ecosystem::PyPI, "Requests", "requests")
        .await
        .unwrap();
    assert_eq!(pkg.id, pkg2.id);
}

#[sqlx::test(migrator = "sp_db::MIGRATOR")]
async fn test_package_find_by_name(pool: sqlx::PgPool) {
    PackageRepo::find_or_create(&pool, Ecosystem::PyPI, "flask", "flask")
        .await
        .unwrap();

    let found = PackageRepo::find_by_name(&pool, Ecosystem::PyPI, "flask")
        .await
        .unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().normalized_name, "flask");

    let not_found = PackageRepo::find_by_name(&pool, Ecosystem::PyPI, "nonexistent")
        .await
        .unwrap();
    assert!(not_found.is_none());
}

#[sqlx::test(migrator = "sp_db::MIGRATOR")]
async fn test_package_list_all(pool: sqlx::PgPool) {
    PackageRepo::find_or_create(&pool, Ecosystem::PyPI, "aaa", "aaa")
        .await
        .unwrap();
    PackageRepo::find_or_create(&pool, Ecosystem::PyPI, "bbb", "bbb")
        .await
        .unwrap();

    let all = PackageRepo::list_all(&pool, Ecosystem::PyPI, 10, 0)
        .await
        .unwrap();
    assert_eq!(all.len(), 2);
    // Should be sorted by normalized_name.
    assert_eq!(all[0].normalized_name, "aaa");
    assert_eq!(all[1].normalized_name, "bbb");
}

#[sqlx::test(migrator = "sp_db::MIGRATOR")]
async fn test_version_find_or_create_pending(pool: sqlx::PgPool) {
    let pkg = PackageRepo::find_or_create(&pool, Ecosystem::PyPI, "requests", "requests")
        .await
        .unwrap();

    let v = VersionRepo::find_or_create_pending(&pool, pkg.id, "2.31.0")
        .await
        .unwrap();
    assert_eq!(v.version, "2.31.0");
    assert_eq!(v.status, AnalysisStatus::Pending);

    // Idempotent: second call returns same row.
    let v2 = VersionRepo::find_or_create_pending(&pool, pkg.id, "2.31.0")
        .await
        .unwrap();
    assert_eq!(v.id, v2.id);
}

#[sqlx::test(migrator = "sp_db::MIGRATOR")]
async fn test_version_update_status(pool: sqlx::PgPool) {
    let pkg = PackageRepo::find_or_create(&pool, Ecosystem::PyPI, "requests", "requests")
        .await
        .unwrap();
    let v = VersionRepo::find_or_create_pending(&pool, pkg.id, "2.31.0")
        .await
        .unwrap();

    VersionRepo::update_status(&pool, v.id, AnalysisStatus::Analyzing)
        .await
        .unwrap();
    let updated = VersionRepo::find(&pool, pkg.id, "2.31.0")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.status, AnalysisStatus::Analyzing);

    VersionRepo::update_status(&pool, v.id, AnalysisStatus::Approved)
        .await
        .unwrap();
    let approved = VersionRepo::find(&pool, pkg.id, "2.31.0")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(approved.status, AnalysisStatus::Approved);
}

#[sqlx::test(migrator = "sp_db::MIGRATOR")]
async fn test_version_list_approved(pool: sqlx::PgPool) {
    let pkg = PackageRepo::find_or_create(&pool, Ecosystem::PyPI, "requests", "requests")
        .await
        .unwrap();

    let v1 = VersionRepo::find_or_create_pending(&pool, pkg.id, "2.30.0")
        .await
        .unwrap();
    VersionRepo::update_status(&pool, v1.id, AnalysisStatus::Approved)
        .await
        .unwrap();

    let v2 = VersionRepo::find_or_create_pending(&pool, pkg.id, "2.31.0")
        .await
        .unwrap();
    VersionRepo::update_status(&pool, v2.id, AnalysisStatus::Rejected)
        .await
        .unwrap();

    VersionRepo::find_or_create_pending(&pool, pkg.id, "2.32.0")
        .await
        .unwrap();

    let approved = VersionRepo::list_approved(&pool, pkg.id).await.unwrap();
    assert_eq!(approved.len(), 1);
    assert_eq!(approved[0].version, "2.30.0");
}

#[sqlx::test(migrator = "sp_db::MIGRATOR")]
async fn test_analysis_record_create_and_find(pool: sqlx::PgPool) {
    let pkg = PackageRepo::find_or_create(&pool, Ecosystem::PyPI, "requests", "requests")
        .await
        .unwrap();
    let v = VersionRepo::find_or_create_pending(&pool, pkg.id, "2.31.0")
        .await
        .unwrap();

    let record = AnalysisRepo::create(
        &pool,
        &sp_db::repos::NewAnalysisRecord {
            package_version_id: v.id,
            analysis_type: "full_scan".to_string(),
            static_scan: Some(serde_json::json!({"findings": []})),
            llm_result: Some(serde_json::json!({"risk_score": 0.05})),
            diff_summary: None,
            verdict: "approved".to_string(),
            risk_score: Some(0.05),
            reasoning: Some("No issues found".to_string()),
            model_used: Some("gemini-2.0-flash".to_string()),
            prompt_tokens: Some(1000),
            completion_tokens: Some(200),
            previous_version_id: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(record.verdict, "approved");
    assert_eq!(record.risk_score, Some(0.05));

    let records = AnalysisRepo::find_by_version(&pool, v.id).await.unwrap();
    assert_eq!(records.len(), 1);

    let latest = AnalysisRepo::find_latest_by_version(&pool, v.id)
        .await
        .unwrap();
    assert!(latest.is_some());
    assert_eq!(latest.unwrap().id, record.id);
}

#[sqlx::test(migrator = "sp_db::MIGRATOR")]
async fn test_audit_log(pool: sqlx::PgPool) {
    let entry = AuditRepo::create(
        &pool,
        "admin",
        "override_verdict",
        Some("package_version"),
        None,
        Some(serde_json::json!({"from": "rejected", "to": "approved"})),
    )
    .await
    .unwrap();

    assert_eq!(entry.actor, "admin");
    assert_eq!(entry.action, "override_verdict");

    let recent = AuditRepo::list_recent(&pool, 10).await.unwrap();
    assert_eq!(recent.len(), 1);
}
