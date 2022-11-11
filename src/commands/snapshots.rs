use std::time::Duration;

use anyhow::Result;
use clap::Parser;
use comfy_table::Cell;
use humantime::format_duration;
use itertools::Itertools;

use super::{bold_cell, bytes, table, table_right_from, RusticConfig};
use crate::backend::DecryptReadBackend;
use crate::repo::{
    DeleteOption, SnapshotFile, SnapshotFilter, SnapshotGroup, SnapshotGroupCriterion,
};

#[derive(Parser)]
pub(super) struct Opts {
    #[clap(flatten, help_heading = "SNAPSHOT FILTER OPTIONS")]
    filter: SnapshotFilter,

    /// Group snapshots by any combination of host,paths,tags
    #[clap(
        long,
        short = 'g',
        value_name = "CRITERION",
        default_value = "host,paths"
    )]
    group_by: SnapshotGroupCriterion,

    /// Show detailed information about snapshots
    #[clap(long)]
    long: bool,

    /// Show snapshots in json format
    #[clap(long, conflicts_with = "long")]
    json: bool,

    /// Show all snapshots instead of summarizing identical follow-up snapshots
    #[clap(long, conflicts_with_all = &["long", "json"])]
    all: bool,

    /// Snapshots to show
    #[clap(value_name = "ID")]
    ids: Vec<String>,
}

pub(super) fn execute(
    be: &impl DecryptReadBackend,
    mut opts: Opts,
    config_file: RusticConfig,
) -> Result<()> {
    config_file.merge_into("snapshot-filter", &mut opts.filter)?;

    let groups = match &opts.ids[..] {
        [] => SnapshotFile::group_from_backend(be, &opts.filter, &opts.group_by)?,
        [id] if id == "latest" => {
            SnapshotFile::group_from_backend(be, &opts.filter, &opts.group_by)?
                .into_iter()
                .map(|(group, mut snaps)| {
                    snaps.sort_unstable();
                    let last_idx = snaps.len() - 1;
                    snaps.swap(0, last_idx);
                    snaps.truncate(1);
                    (group, snaps)
                })
                .collect::<Vec<_>>()
        }
        _ => vec![(
            SnapshotGroup::default(),
            SnapshotFile::from_ids(be, &opts.ids)?,
        )],
    };

    if opts.json {
        let mut stdout = std::io::stdout();
        serde_json::to_writer_pretty(&mut stdout, &groups)?;
        return Ok(());
    }

    for (group, mut snapshots) in groups {
        if !group.is_empty() {
            println!("\nsnapshots for {group}");
        }
        snapshots.sort_unstable();
        let count = snapshots.len();

        if opts.long {
            for snap in snapshots {
                display_snap(snap);
            }
        } else {
            let snap_to_table = |(sn, count): (SnapshotFile, usize)| {
                let tags = sn.tags.formatln();
                let paths = sn.paths.formatln();
                let time = sn.time.format("%Y-%m-%d %H:%M:%S");
                let (files, dirs, size) = match &sn.summary {
                    Some(s) => (
                        s.total_files_processed.to_string(),
                        s.total_dirs_processed.to_string(),
                        bytes(s.total_bytes_processed),
                    ),
                    None => ("?".to_string(), "?".to_string(), "?".to_string()),
                };
                let id = match count {
                    0 => format!("{}", sn.id),
                    count => format!("{} (+{})", sn.id, count),
                };
                [
                    id,
                    time.to_string(),
                    sn.hostname,
                    tags,
                    paths,
                    files,
                    dirs,
                    size,
                ]
            };

            let mut table = table_right_from(
                5,
                [
                    "ID", "Time", "Host", "Tags", "Paths", "Files", "Dirs", "Size",
                ],
            );

            let snapshots: Vec<_> = snapshots
                .into_iter()
                .group_by(|sn| if opts.all { sn.id } else { sn.tree })
                .into_iter()
                .map(|(_, mut g)| (g.next().unwrap(), g.count()))
                .map(snap_to_table)
                .collect();
            table.add_rows(snapshots);
            println!("{table}");
        }
        println!("{} snapshot(s)", count);
    }

    Ok(())
}

fn display_snap(sn: SnapshotFile) {
    let mut table = table();

    let mut add_entry = |title: &str, value: String| {
        table.add_row([bold_cell(title), Cell::new(value)]);
    };

    add_entry("Snapshot", sn.id.to_hex());
    // note that if original was not set, it is set to sn.id by the load process
    if sn.original != Some(sn.id) {
        add_entry("Original ID", sn.original.unwrap().to_hex());
    }
    add_entry("Time", sn.time.format("%Y-%m-%d %H:%M:%S").to_string());
    add_entry("Host", sn.hostname);
    add_entry("Tags", sn.tags.formatln());
    let delete = match sn.delete {
        DeleteOption::NotSet => "not set".to_string(),
        DeleteOption::Never => "never".to_string(),
        DeleteOption::After(t) => format!("after {}", t.format("%Y-%m-%d %H:%M:%S")),
    };
    add_entry("Delete", delete);
    add_entry("Paths", sn.paths.formatln());
    let parent = match sn.parent {
        None => "no parent snapshot".to_string(),
        Some(p) => p.to_hex(),
    };
    add_entry("Parent", parent);
    if let Some(summary) = sn.summary {
        add_entry("", "".to_string());
        add_entry("Command", summary.command);

        let source = format!(
            "files: {} / dirs: {} / size: {}",
            summary.total_files_processed,
            summary.total_dirs_processed,
            bytes(summary.total_bytes_processed)
        );
        add_entry("Source", source);
        add_entry("", "".to_string());

        let files = format!(
            "new: {:>10} / changed: {:>10} / unchanged: {:>10}",
            summary.files_new, summary.files_changed, summary.files_unmodified,
        );
        add_entry("Files", files);

        let trees = format!(
            "new: {:>10} / changed: {:>10} / unchanged: {:>10}",
            summary.dirs_new, summary.dirs_changed, summary.dirs_unmodified,
        );
        add_entry("Dirs", trees);
        add_entry("", "".to_string());

        let written = format!(
            "data:  {:>10} blobs / raw: {:>10} / packed: {:>10}\n\
            tree:  {:>10} blobs / raw: {:>10} / packed: {:>10}\n\
            total: {:>10} blobs / raw: {:>10} / packed: {:>10}",
            summary.data_blobs,
            bytes(summary.data_added_files),
            bytes(summary.data_added_files_packed),
            summary.tree_blobs,
            bytes(summary.data_added_trees),
            bytes(summary.data_added_trees_packed),
            summary.tree_blobs + summary.data_blobs,
            bytes(summary.data_added),
            bytes(summary.data_added_packed),
        );
        add_entry("Added to repo", written);

        let duration = format!(
            "backup start: {} / backup end: {} / backup duration: {}\n\
            total duration: {}",
            summary.backup_start.format("%Y-%m-%d %H:%M:%S"),
            summary.backup_end.format("%Y-%m-%d %H:%M:%S"),
            format_duration(Duration::from_secs_f64(summary.backup_duration)),
            format_duration(Duration::from_secs_f64(summary.total_duration))
        );
        add_entry("Duration", duration);
    }
    println!("{table}");
    println!();
}
