use siori::app::{
    FileEntry, FileStatus, PendingDiscard, PendingDiscardAction, execute_pending_discard_with,
};
use std::path::Path;

fn file(path: &str, status: FileStatus, staged: bool) -> FileEntry {
    FileEntry {
        path: path.to_string(),
        status,
        staged,
        diff_stats: None,
    }
}

#[test]
fn tracked_modified_is_restored() {
    let pending = PendingDiscard::for_file(&file("src/main.rs", FileStatus::Modified, false))
        .expect("tracked file should be discardable");

    assert_eq!(pending.action, PendingDiscardAction::RestoreTracked);
}

#[test]
fn tracked_deleted_is_restored() {
    let pending = PendingDiscard::for_file(&file("src/old.rs", FileStatus::Deleted, false))
        .expect("deleted file should be discardable");

    assert_eq!(pending.action, PendingDiscardAction::RestoreTracked);
}

#[test]
fn untracked_file_moves_to_trash() {
    let pending =
        PendingDiscard::for_file(&file("notes.txt", FileStatus::Untracked, false)).unwrap();

    assert_eq!(pending.action, PendingDiscardAction::TrashUntracked);
}

#[test]
fn untracked_directory_moves_to_trash() {
    let pending =
        PendingDiscard::for_file(&file("scratch/", FileStatus::Untracked, false)).unwrap();

    assert_eq!(pending.action, PendingDiscardAction::TrashUntracked);
}

#[test]
fn staged_items_are_rejected_before_confirm() {
    let error = PendingDiscard::for_file(&file("src/main.rs", FileStatus::Added, true))
        .expect_err("staged files must be rejected");

    assert_eq!(error, "Unstage file first (Space)");
}

#[test]
fn restore_action_uses_restore_executor() {
    let pending = PendingDiscard {
        path: "src/main.rs".to_string(),
        action: PendingDiscardAction::RestoreTracked,
    };
    let mut restore_calls = 0;
    let mut trash_calls = 0;

    let result = execute_pending_discard_with(
        Path::new("/repo"),
        &pending,
        |repo_path, path| {
            restore_calls += 1;
            assert_eq!(repo_path, Path::new("/repo"));
            assert_eq!(path, "src/main.rs");
            Ok(())
        },
        |_, _| {
            trash_calls += 1;
            Ok(())
        },
    )
    .expect("restore should succeed");

    assert_eq!(result, "Discarded: src/main.rs");
    assert_eq!(restore_calls, 1);
    assert_eq!(trash_calls, 0);
}

#[test]
fn trash_action_uses_trash_executor() {
    let pending = PendingDiscard {
        path: "notes.txt".to_string(),
        action: PendingDiscardAction::TrashUntracked,
    };
    let mut restore_calls = 0;
    let mut trash_calls = 0;

    let result = execute_pending_discard_with(
        Path::new("/repo"),
        &pending,
        |_, _| {
            restore_calls += 1;
            Ok(())
        },
        |repo_path, path| {
            trash_calls += 1;
            assert_eq!(repo_path, Path::new("/repo"));
            assert_eq!(path, "notes.txt");
            Ok(())
        },
    )
    .expect("trash should succeed");

    assert_eq!(result, "Moved to trash: notes.txt");
    assert_eq!(restore_calls, 0);
    assert_eq!(trash_calls, 1);
}

#[test]
fn trash_failure_is_returned_without_fallback_delete() {
    let pending = PendingDiscard {
        path: "notes.txt".to_string(),
        action: PendingDiscardAction::TrashUntracked,
    };

    let error = execute_pending_discard_with(
        Path::new("/repo"),
        &pending,
        |_, _| Ok(()),
        |_, _| Err("Move to trash failed: permission denied".to_string()),
    )
    .expect_err("trash failures should bubble up");

    assert_eq!(error, "Move to trash failed: permission denied");
}
