//! Saved conversations: list/load/save/delete/rename/delete-all, the 30-thread cap and the
//! per-project folder name shared with the old Node server.

use contextual_core::error::ErrorCode;
use contextual_core::threads::{MAX_THREADS, ThreadStore, ThreadSummary, project_hash};
use pretty_assertions::assert_eq;
use serde_json::json;

const ROOT: &str = "/home/me/project";

fn summary(id: &str, updated_at: &str) -> ThreadSummary {
    ThreadSummary {
        id: id.into(),
        title: format!("Title {id}"),
        updated_at: updated_at.into(),
        turns: 1,
    }
}

#[test]
fn project_hash_matches_node() {
    // `createHash('sha1').update(root).digest('hex').slice(0, 16)` from node.
    assert_eq!(project_hash("/home/me/project"), "aac50b7132bfdf32");
    assert_eq!(project_hash("/mnt/c/src/x"), "da4792781e107f59");
    assert_eq!(project_hash("/tmp/é ü"), "8573d77aaefac096");
}

#[tokio::test]
async fn save_list_load_rename_delete() {
    let dir = tempfile::tempdir().unwrap();
    let store = ThreadStore::new(dir.path());
    assert_eq!(store.list(ROOT).await.unwrap(), vec![]);

    let body = json!({ "title": "First", "messages": [{ "role": "user", "content": "hi" }] });
    store
        .save(ROOT, "t-1", Some("First"), 2, &body)
        .await
        .unwrap();
    store
        .save_summary(
            ROOT,
            summary("t-2", "2099-01-01T00:00:00.000Z"),
            &json!({"title": "Second"}),
        )
        .await
        .unwrap();

    // Files live where the old server put them: <dir>/<hash>/<id>.json + index.json.
    let project = dir.path().join("aac50b7132bfdf32");
    assert!(project.join("t-1.json").is_file());
    assert!(project.join("index.json").is_file());
    assert_eq!(store.project_dir(ROOT).unwrap(), project);

    let list = store.list(ROOT).await.unwrap();
    assert_eq!(
        list.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
        ["t-2", "t-1"]
    );
    assert_eq!(list[1].title, "First");
    assert_eq!(list[1].turns, 2);
    assert_eq!(store.load(ROOT, "t-1").await.unwrap(), body);

    // Index wire format (camelCase, like index.json from the old server).
    let index: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(project.join("index.json")).unwrap())
            .unwrap();
    assert_eq!(
        index[0],
        json!({"id":"t-2","title":"Title t-2","updatedAt":"2099-01-01T00:00:00.000Z","turns":1})
    );

    store.rename(ROOT, "t-1", "  Renamed  ").await.unwrap();
    assert_eq!(store.load(ROOT, "t-1").await.unwrap()["title"], "Renamed");
    assert_eq!(store.list(ROOT).await.unwrap()[1].title, "Renamed");
    let e = store.rename(ROOT, "t-1", "   ").await.unwrap_err();
    assert_eq!(e.message, "The name cannot be empty.");
    let e = store.rename(ROOT, "nope", "x").await.unwrap_err();
    assert_eq!(
        (e.code, e.message.as_str()),
        (ErrorCode::NotFound, "Conversation not found.")
    );

    store.delete(ROOT, "t-2").await.unwrap();
    assert_eq!(store.list(ROOT).await.unwrap().len(), 1);
    assert!(!project.join("t-2.json").exists());
    let e = store.load(ROOT, "t-2").await.unwrap_err();
    assert_eq!(e.message, "Conversation not found.");

    // Other projects are separate.
    assert!(store.list("/home/me/other").await.unwrap().is_empty());

    assert_eq!(store.delete_all(ROOT).await.unwrap(), 1);
    assert!(store.list(ROOT).await.unwrap().is_empty());
    assert_eq!(
        std::fs::read_to_string(project.join("index.json")).unwrap(),
        "[]"
    );
}

#[tokio::test]
async fn keeps_the_newest_thirty() {
    let dir = tempfile::tempdir().unwrap();
    let store = ThreadStore::new(dir.path());
    for i in 0..(MAX_THREADS + 3) {
        let at = format!("2024-01-01T00:00:{i:02}.000Z");
        store
            .save_summary(ROOT, summary(&format!("t{i}"), &at), &json!({ "i": i }))
            .await
            .unwrap();
    }
    let list = store.list(ROOT).await.unwrap();
    assert_eq!(list.len(), MAX_THREADS);
    assert_eq!(list[0].id, "t32");
    assert_eq!(list.last().unwrap().id, "t3");
    let project = store.project_dir(ROOT).unwrap();
    for gone in ["t0", "t1", "t2"] {
        assert!(!project.join(format!("{gone}.json")).exists());
    }
    // Saving an existing id replaces its entry instead of adding one.
    store.save(ROOT, "t5", None, 9, &json!({})).await.unwrap();
    let list = store.list(ROOT).await.unwrap();
    assert_eq!(list.len(), MAX_THREADS);
    assert_eq!(
        (list[0].id.as_str(), list[0].title.as_str(), list[0].turns),
        ("t5", "Conversation", 9)
    );
}

#[tokio::test]
async fn rejects_bad_ids_and_tolerates_a_broken_index() {
    let dir = tempfile::tempdir().unwrap();
    let store = ThreadStore::new(dir.path());
    for bad in ["", "../x", "a/b", "a.b", &"x".repeat(65)] {
        let e = store.load(ROOT, bad).await.unwrap_err();
        assert_eq!(
            (e.code, e.message.as_str()),
            (ErrorCode::InvalidInput, "Invalid thread id."),
            "{bad}"
        );
    }
    let project = store.project_dir(ROOT).unwrap();
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("index.json"), "{not json").unwrap();
    assert!(store.list(ROOT).await.unwrap().is_empty());
    std::fs::write(project.join("index.json"), r#"{"a":1}"#).unwrap();
    assert!(store.list(ROOT).await.unwrap().is_empty());
    store
        .save(ROOT, "ok", Some(&"é".repeat(300)), 0, &json!(null))
        .await
        .unwrap();
    let list = store.list(ROOT).await.unwrap();
    assert_eq!(list[0].title.chars().count(), 200);
}
