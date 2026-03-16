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

fn collect_discard_targets(files: &[FileEntry]) -> Vec<PendingDiscard> {
    files
        .iter()
        .filter(|f| !f.staged)
        .filter_map(|f| PendingDiscard::for_file(f).ok())
        .collect()
}

#[test]
fn only_unstaged_files_collected() {
    let files = vec![
        file("staged.rs", FileStatus::Modified, true),
        file("changed.rs", FileStatus::Modified, false),
        file("new.txt", FileStatus::Untracked, false),
    ];
    let targets = collect_discard_targets(&files);
    assert_eq!(targets.len(), 2);
    assert_eq!(targets[0].path, "changed.rs");
    assert_eq!(targets[1].path, "new.txt");
}

#[test]
fn all_staged_returns_empty() {
    let files = vec![
        file("a.rs", FileStatus::Added, true),
        file("b.rs", FileStatus::Modified, true),
    ];
    let targets = collect_discard_targets(&files);
    assert!(targets.is_empty());
}

#[test]
fn mixed_tracked_untracked_assigns_correct_actions() {
    let files = vec![
        file("modified.rs", FileStatus::Modified, false),
        file("deleted.rs", FileStatus::Deleted, false),
        file("untracked.txt", FileStatus::Untracked, false),
    ];
    let targets = collect_discard_targets(&files);
    assert_eq!(targets.len(), 3);
    assert_eq!(targets[0].action, PendingDiscardAction::RestoreTracked);
    assert_eq!(targets[1].action, PendingDiscardAction::RestoreTracked);
    assert_eq!(targets[2].action, PendingDiscardAction::TrashUntracked);
}

#[test]
fn bulk_discard_counts_success_and_failure() {
    let targets = vec![
        PendingDiscard {
            path: "ok1.rs".to_string(),
            action: PendingDiscardAction::RestoreTracked,
        },
        PendingDiscard {
            path: "fail.rs".to_string(),
            action: PendingDiscardAction::RestoreTracked,
        },
        PendingDiscard {
            path: "ok2.txt".to_string(),
            action: PendingDiscardAction::TrashUntracked,
        },
    ];

    let mut success = 0usize;
    let mut failure = 0usize;

    for pending in &targets {
        let result = execute_pending_discard_with(
            Path::new("/repo"),
            pending,
            |_, path| {
                if path == "fail.rs" {
                    Err("restore failed".to_string())
                } else {
                    Ok(())
                }
            },
            |_, _| Ok(()),
        );
        match result {
            Ok(_) => success += 1,
            Err(_) => failure += 1,
        }
    }

    assert_eq!(success, 2);
    assert_eq!(failure, 1);
}
