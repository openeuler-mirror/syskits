/*
 * Copyright(c) 2022-2025 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2.
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2.
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 * KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 * NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 * See the Mulan PSL v2 for more details.
 */

use crate::args::{TailFollowMode, TailOptions};
use crate::follow::files::{FileHandling, PathData};
use crate::paths::{TailInput, TailInputKind, TailMetadataExt, TailPathExt};
use crate::{platform, text};
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTResult, CtSimpleError, set_ct_exit_code};
use ctcore::ct_show_error;
use notify::{RecommendedWatcher, RecursiveMode, Watcher, WatcherKind};
use std::collections::HashMap;
use std::fs::Metadata;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, channel};
use std::time::Duration;

pub struct WatcherRx {
    watcher: Box<dyn Watcher>,
    receiver: Receiver<Result<notify::Event, notify::Error>>,
}

impl WatcherRx {
    fn new(
        watcher: Box<dyn Watcher>,
        receiver: Receiver<Result<notify::Event, notify::Error>>,
    ) -> Self {
        Self { watcher, receiver }
    }

    /// Wrapper for `notify::Watcher::watch` to also add the parent directory of `path` if necessary.
    fn watch_with_parent(&mut self, path: &Path) -> CTResult<()> {
        let mut path = path.to_owned();
        #[cfg(target_os = "linux")]
        if path.is_file() {
            /*
            NOTE: Using the parent directory instead of the file is a workaround.
            This workaround follows the recommendation of the notify crate authors:
            > On some platforms, if the `path` is renamed or removed while being watched, behavior may
            > be unexpected. See discussions in [#165] and [#166]. If less surprising behavior is wanted
            > one may non-recursively watch the _parent_ directory as well and manage related events.
            NOTE: Adding both: file and parent results in duplicate/wrong events.
            Tested for notify::InotifyWatcher and for notify::PollWatcher.
            */
            if let Some(parent) = path.parent() {
                if parent.is_dir() {
                    path = parent.to_owned();
                } else {
                    path = PathBuf::from(".");
                }
            } else {
                return Err(CtSimpleError::new(
                    1,
                    format!("cannot watch parent directory of {}", path.display()),
                ));
            };
        }
        if path.is_relative() {
            path = path.canonicalize()?;
        }

        // for syscalls: 2x "inotify_add_watch" ("filename" and ".") and 1x "inotify_rm_watch"
        self.watch(&path, RecursiveMode::NonRecursive)?;
        Ok(())
    }

    fn watch(&mut self, path: &Path, mode: RecursiveMode) -> CTResult<()> {
        self.watcher
            .watch(path, mode)
            .map_err(|err| CtSimpleError::new(1, err.to_string()))
    }

    fn unwatch(&mut self, path: &Path) -> CTResult<()> {
        self.watcher
            .unwatch(path)
            .map_err(|err| CtSimpleError::new(1, err.to_string()))
    }
}

pub struct Observer {
    /// Whether --retry was given on the command line
    pub retry: bool,

    /// The [`TailFollowMode`]
    pub follow: Option<TailFollowMode>,

    /// Indicates whether to use the fallback `polling` method instead of the
    /// platform specific event driven method. Since `use_polling` is subject to
    /// change during runtime it is moved out of [`TailSettings`].
    pub use_polling: bool,

    pub watcher_rx: Option<WatcherRx>,
    pub orphans: Vec<PathBuf>,
    pub files: FileHandling,
    pending_renames: HashMap<usize, PathBuf>,

    pub pid: platform::Pid,
}

impl Observer {
    /// Directory removal may race with file removal notifications (e.g. `rm -r`).
    /// Give the parent path a brief window to disappear before deciding.
    fn orphan_after_brief_wait(path: &Path) -> bool {
        if path.is_orphan() {
            return true;
        }

        for _ in 0..5 {
            std::thread::sleep(Duration::from_millis(20));
            if path.is_orphan() {
                return true;
            }
        }

        false
    }

    pub fn new(
        retry: bool,
        follow: Option<TailFollowMode>,
        use_polling: bool,
        files: FileHandling,
        pid: platform::Pid,
    ) -> Self {
        let pid = if platform::supports_pid_checks(pid) {
            pid
        } else {
            0
        };

        Self {
            retry,
            follow,
            use_polling,
            watcher_rx: None,
            orphans: Vec::new(),
            files,
            pending_renames: HashMap::new(),
            pid,
        }
    }

    pub fn from(options: &TailOptions) -> Self {
        Self::new(
            options.retry,
            options.follow,
            options.use_polling,
            FileHandling::from(options),
            options.pid,
        )
    }

    pub fn add_path(
        &mut self,
        path: &Path,
        display_name: &str,
        reader: Option<Box<dyn BufRead>>,
        update_last: bool,
    ) -> CTResult<()> {
        if self.follow.is_some() {
            let path = if path.is_relative() {
                std::env::current_dir()?.join(path)
            } else {
                path.to_owned()
            };
            let metadata = path.metadata().ok();
            self.files.insert(
                &path,
                PathData::new(reader, metadata, display_name),
                update_last,
            );
        }

        Ok(())
    }

    pub fn add_stdin(
        &mut self,
        display_name: &str,
        reader: Option<Box<dyn BufRead>>,
        update_last: bool,
    ) -> CTResult<()> {
        if self.follow == Some(TailFollowMode::Descriptor) {
            return self.add_path(
                &PathBuf::from(text::TAIL_DEV_STDIN),
                display_name,
                reader,
                update_last,
            );
        }

        Ok(())
    }

    pub fn add_bad_path(
        &mut self,
        path: &Path,
        display_name: &str,
        update_last: bool,
    ) -> CTResult<()> {
        if self.retry && self.follow.is_some() {
            return self.add_path(path, display_name, None, update_last);
        }

        Ok(())
    }

    pub fn start(&mut self, options: &TailOptions) -> CTResult<()> {
        if options.follow.is_none() {
            return Ok(());
        }

        let (tx, rx) = channel();

        /*
        Watcher is implemented per platform using the best implementation available on that
        platform. In addition to such event driven implementations, a polling implementation
        is also provided that should work on any platform.
        Linux / Android: inotify
        Windows: ReadDirectoryChangesWatcher
        Fallback: polling every n seconds
        */

        let watcher: Box<dyn Watcher>;
        let watcher_config = notify::Config::default()
            .with_poll_interval(options.sleep_sec)
            /*
            NOTE: By enabling compare_contents, performance will be significantly impacted
            as all files will need to be read and hashed at each `poll_interval`.
            However, this is necessary to pass: "gnu/tests/tail-2/F-vs-rename.sh"
            */
            .with_compare_contents(true);
        if self.use_polling || RecommendedWatcher::kind() == WatcherKind::PollWatcher {
            self.use_polling = true; // We have to use polling because there's no supported backend
            watcher = Box::new(notify::PollWatcher::new(tx, watcher_config).unwrap());
        } else {
            let tx_clone = tx.clone();
            match notify::RecommendedWatcher::new(tx, notify::Config::default()) {
                Ok(w) => watcher = Box::new(w),
                Err(e) if e.to_string().starts_with("Too many open files") => {
                    /*
                    NOTE: This ErrorKind is `Uncategorized`, but it is not recommended
                    to match an error against `Uncategorized`
                    NOTE: Could be tested with decreasing `max_user_instances`, e.g.:
                    `sudo sysctl fs.inotify.max_user_instances=64`
                    */
                    ct_show_error!(
                        "{} cannot be used, reverting to polling: Too many open files",
                        text::TAIL_BACKEND
                    );
                    set_ct_exit_code(1);
                    self.use_polling = true;
                    watcher = Box::new(notify::PollWatcher::new(tx_clone, watcher_config).unwrap());
                }
                Err(e) => return Err(CtSimpleError::new(1, e.to_string())),
            };
        }

        self.watcher_rx = Some(WatcherRx::new(watcher, rx));
        self.init_files(&options.inputs)?;

        Ok(())
    }

    pub fn follow_descriptor(&self) -> bool {
        self.follow == Some(TailFollowMode::Descriptor)
    }

    pub fn follow_name(&self) -> bool {
        self.follow == Some(TailFollowMode::Name)
    }

    pub fn follow_descriptor_retry(&self) -> bool {
        self.follow_descriptor() && self.retry
    }

    pub fn follow_name_retry(&self) -> bool {
        self.follow_name() && self.retry
    }

    fn tracked_path_for_event(&self, event: &notify::Event) -> Option<PathBuf> {
        use notify::EventKind;
        use notify::event::{ModifyKind, RemoveKind, RenameMode};

        let event_path = event.paths.first()?;
        if self.files.contains_key(event_path) {
            return Some(event_path.clone());
        }

        // For follow-name + retry, some backends report only the parent
        // directory for directory-removal notifications.
        if self.follow_name_retry()
            && matches!(
                event.kind,
                EventKind::Remove(RemoveKind::Folder | RemoveKind::Any)
            )
        {
            let mut candidates = self
                .files
                .keys()
                .filter(|path| path.parent().is_some_and(|parent| parent == event_path));
            if let Some(first) = candidates.next() {
                // If several tracked files share the same parent, do not guess.
                // Ambiguous cases are handled by periodic reconcile.
                if candidates.next().is_none() {
                    return Some(first.clone());
                }
            }
        }

        if self.follow_descriptor()
            && matches!(
                event.kind,
                EventKind::Modify(ModifyKind::Name(RenameMode::To))
            )
        {
            return event
                .tracker()
                .and_then(|tracker| self.pending_renames.get(&tracker).cloned());
        }

        None
    }

    fn relocate_descriptor_path(&mut self, old_path: &Path, new_path: &Path) -> CTResult<()> {
        let update_last = self.files.get_last().is_some_and(|last| last == old_path);
        let new_data = PathData::from_other_with_path(self.files.remove(old_path), new_path);
        self.files.insert(new_path, new_data, update_last);

        if let Some(watcher_rx) = self.watcher_rx.as_mut() {
            let _ = watcher_rx.unwatch(old_path);
            watcher_rx.watch_with_parent(new_path)?;
        }

        Ok(())
    }

    fn init_files(&mut self, inputs: &Vec<TailInput>) -> CTResult<()> {
        if let Some(watcher_rx) = &mut self.watcher_rx {
            for input in inputs {
                match input.kind() {
                    TailInputKind::Stdin => continue,
                    TailInputKind::File(path) => {
                        #[cfg(all(unix, not(target_os = "linux")))]
                        if !path.is_file() {
                            continue;
                        }
                        let mut path = path.clone();
                        if path.is_relative() {
                            path = std::env::current_dir()?.join(path);
                        }

                        if path.is_tailable() {
                            // Add existing regular files to `Watcher` (InotifyWatcher).
                            watcher_rx.watch_with_parent(&path)?;
                        } else if !path.is_orphan() {
                            // If `path` is not a tailable file, add its parent to `Watcher`.
                            watcher_rx
                                .watch(path.parent().unwrap(), RecursiveMode::NonRecursive)?;
                        } else {
                            // If there is no parent, add `path` to `orphans`.
                            self.orphans.push(path);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    #[allow(clippy::cognitive_complexity)]
    fn handle_event(
        &mut self,
        event: &notify::Event,
        options: &TailOptions,
    ) -> CTResult<Vec<PathBuf>> {
        use notify::event::*;

        let event_path = event.paths.first().unwrap();
        let mut paths: Vec<PathBuf> = vec![];
        let Some(tracked_path) = self.tracked_path_for_event(event) else {
            return Ok(paths);
        };
        let display_name = self.files.get(&tracked_path).display_name.clone();

        if self.follow_descriptor()
            && matches!(
                event.kind,
                EventKind::Modify(ModifyKind::Name(RenameMode::To))
            )
            && tracked_path != *event_path
        {
            if let Some(tracker) = event.tracker() {
                self.pending_renames.remove(&tracker);
            }
            self.relocate_descriptor_path(&tracked_path, event_path)?;
            paths.push(event_path.clone());
            return Ok(paths);
        }

        match event.kind {
            EventKind::Modify(ModifyKind::Metadata(MetadataKind::Any | MetadataKind::WriteTime))

            // | EventKind::Access(AccessKind::Close(AccessMode::Write))
            | EventKind::Create(CreateKind::File | CreateKind::Folder | CreateKind::Any)
            | EventKind::Modify(ModifyKind::Data(DataChange::Any))
            | EventKind::Modify(ModifyKind::Name(RenameMode::To)) => {
                if let Ok(new_md) = event_path.metadata() {
                    let is_tailable = new_md.is_tailable();
                    let pd = self.files.get(&tracked_path);
                    if let Some(old_md) = &pd.metadata {
                        if is_tailable {
                            // We resume tracking from the start of the file,
                            // assuming it has been truncated to 0. This mimics GNU's `tail`
                            // behavior and is the usual truncation operation for log self.files.
                            if !old_md.is_tailable() {
                                ct_show_error!( "{} has become accessible", display_name.quote());
                                self.files.update_reader(&tracked_path)?;
                            } else if pd.reader.is_none() {
                                ct_show_error!( "{} has appeared;  following new file", display_name.quote());
                                self.files.update_reader(&tracked_path)?;
                            } else if event.kind == EventKind::Modify(ModifyKind::Name(RenameMode::To))
                                || (self.use_polling
                                && !old_md.file_id_eq(&new_md)) {
                                ct_show_error!( "{} has been replaced;  following new file", display_name.quote());
                                self.files.update_reader(&tracked_path)?;
                            } else if old_md.got_truncated(&new_md)? {
                                ct_show_error!("{}: file truncated", display_name);
                                self.files.update_reader(&tracked_path)?;
                            }
                            paths.push(tracked_path.clone());
                        } else if !is_tailable && old_md.is_tailable() {
                            if pd.reader.is_some() {
                                self.files.reset_reader(&tracked_path);
                            } else {
                                ct_show_error!(
                                        "{} has been replaced with an untailable file",
                                        display_name.quote()
                                    );
                            }
                        }
                    } else if is_tailable {
                        ct_show_error!( "{} has appeared;  following new file", display_name.quote());
                        self.files.update_reader(&tracked_path)?;
                        paths.push(tracked_path.clone());
                    } else if options.retry {
                        if self.follow_descriptor() {
                            ct_show_error!(
                                        "{} has been replaced with an untailable file; giving up on this name",
                                        display_name.quote()
                                    );
                            if let Some(watcher_rx) = self.watcher_rx.as_mut() {
                                let _ = watcher_rx.watcher.unwatch(&tracked_path);
                            }
                            self.files.remove(&tracked_path);
                            if self.files.no_files_remaining(options) {
                                return Err(CtSimpleError::new(1, text::TAIL_NO_FILES_REMAINING));
                            }
                        } else {
                            ct_show_error!(
                                        "{} has been replaced with an untailable file",
                                        display_name.quote()
                                    );
                        }
                    }
                    self.files.update_metadata(&tracked_path, Some(new_md));
                }
            }
            EventKind::Remove(RemoveKind::File | RemoveKind::Folder | RemoveKind::Any)

            // | EventKind::Modify(ModifyKind::Name(RenameMode::Any))
            | EventKind::Modify(ModifyKind::Name(RenameMode::From)) => {
                if self.follow_descriptor() {
                    if let Some(tracker) = event.tracker() {
                        self.pending_renames.insert(tracker, tracked_path.clone());
                    }
                }
                if self.follow_name() {
                    if options.retry {
                        if !tracked_path.exists() {
                            if let Some(old_md) = self.files.get_mut_metadata(&tracked_path) {
                                if old_md.is_tailable()
                                    && self.files.get(&tracked_path).reader.is_some()
                                {
                                    ct_show_error!(
                                            "{} {}: {}",
                                            display_name.quote(),
                                            text::TAIL_BECOME_INACCESSIBLE,
                                            text::TAIL_NO_SUCH_FILE
                                        );
                                }
                            }
                            if Self::orphan_after_brief_wait(&tracked_path)
                                && !self.orphans.contains(&tracked_path)
                            {
                                ct_show_error!("directory containing watched file was removed");
                                ct_show_error!(
                                        "{} cannot be used, reverting to polling",
                                        text::TAIL_BACKEND
                                    );
                                self.use_polling = true;
                                self.orphans.push(tracked_path.clone());
                                if let Some(rx) = self.watcher_rx.as_mut() {
                                    let _ = rx.unwatch(event_path);
                                }
                            }
                            self.files.reset_reader(&tracked_path);
                        }
                    } else {
                        ct_show_error!("{}: {}", display_name, text::TAIL_NO_SUCH_FILE);
                        if !self.files.files_remaining() && self.use_polling {
                            // NOTE: GNU's tail exits here for `---disable-inotify`
                            return Err(CtSimpleError::new(1, text::TAIL_NO_FILES_REMAINING));
                        }
                        self.files.reset_reader(&tracked_path);
                    }
                } else if self.follow_descriptor_retry()
                    && !matches!(event.kind, EventKind::Modify(ModifyKind::Name(RenameMode::From)))
                {
                    // --retry only effective for the initial open
                    if let Some(watcher_rx) = self.watcher_rx.as_mut() {
                        let _ = watcher_rx.unwatch(&tracked_path);
                    }
                    self.files.remove(&tracked_path);
                } else if self.use_polling && event.kind == EventKind::Remove(RemoveKind::Any) {
                    /*
                    BUG: The watched file was removed. Since we're using Polling, this
                    could be a rename. We can't tell because `notify::PollWatcher` doesn't
                    recognize renames properly.
                    Ideally we want to call seek to offset 0 on the file handle.
                    But because we only have access to `PathData::reader` as `BufRead`,
                    we cannot seek to 0 with `BufReader::seek_relative`.
                    Also because we don't have the new name, we cannot work around this
                    by simply reopening the file.
                    */
                }
            }
            EventKind::Modify(ModifyKind::Name(RenameMode::Both)) => {
                /*
                NOTE: For `tail -f a`, keep tracking additions to b after `mv a b`
                (gnu/tests/tail-2/descriptor-vs-rename.sh)
                NOTE: The File/BufReader doesn't need to be updated.
                However, we need to update our `files.map`.
                This can only be done for inotify, because this EventKind does not
                trigger for the PollWatcher.
                BUG: As a result, there's a bug if polling is used:
                $ tail -f file_a ---disable-inotify
                $ mv file_a file_b
                $ echo A >> file_b
                $ echo A >> file_a
                The last append to file_a is printed, however this shouldn't be because
                after the "mv" tail should only follow "file_b".
                TODO: [2022-05; jhscheer] add test for this bug
                */

                if self.follow_descriptor() {
                    if let Some(tracker) = event.tracker() {
                        self.pending_renames.remove(&tracker);
                    }
                    let new_path = event.paths.last().unwrap();
                    paths.push(new_path.clone());
                    self.relocate_descriptor_path(&tracked_path, new_path)?;
                }
            }
            _ => {}
        }
        Ok(paths)
    }

    fn reconcile_follow_name_retry(&mut self, verbose: bool) -> CTResult<bool> {
        if !self.follow_name_retry() {
            return Ok(false);
        }

        let mut read_some = false;
        let tracked_paths = self.files.keys().cloned().collect::<Vec<_>>();
        let mut inaccessible_paths = Vec::<PathBuf>::new();
        let mut orphaned_readerless_paths = Vec::<PathBuf>::new();
        let mut newly_appeared_paths = Vec::<PathBuf>::new();
        let mut replaced_paths = Vec::<(PathBuf, Metadata)>::new();
        let mut truncated_paths = Vec::<(PathBuf, Metadata)>::new();
        let mut unchanged_paths = Vec::<(PathBuf, Metadata)>::new();
        let mut steady_reader_paths = Vec::<PathBuf>::new();

        for path in tracked_paths {
            let reader_exists = self.files.get(&path).reader.is_some();

            if !reader_exists {
                if path.exists() {
                    newly_appeared_paths.push(path);
                } else if path.is_orphan() {
                    orphaned_readerless_paths.push(path);
                }
                continue;
            }

            if !path.exists() {
                inaccessible_paths.push(path);
                continue;
            }

            let Ok(new_md) = path.metadata() else {
                continue;
            };

            let mut replaced = false;
            let mut truncated = false;

            if let Some(old_md) = self.files.get(&path).metadata.as_ref() {
                replaced = !old_md.file_id_eq(&new_md);
                if !replaced {
                    truncated = old_md.got_truncated(&new_md)?;
                }
            }

            if replaced {
                replaced_paths.push((path, new_md));
            } else if truncated {
                truncated_paths.push((path, new_md));
            } else {
                steady_reader_paths.push(path.clone());
                unchanged_paths.push((path, new_md));
            }
        }

        for path in inaccessible_paths {
            let display_name = self.files.get(&path).display_name.clone();
            ct_show_error!(
                "{} {}: {}",
                display_name.quote(),
                text::TAIL_BECOME_INACCESSIBLE,
                text::TAIL_NO_SUCH_FILE
            );
            if path.is_orphan() && !self.orphans.contains(&path) {
                ct_show_error!("directory containing watched file was removed");
                ct_show_error!(
                    "{} cannot be used, reverting to polling",
                    text::TAIL_BACKEND
                );
                self.use_polling = true;
                self.orphans.push(path.clone());
                if let Some(rx) = self.watcher_rx.as_mut() {
                    let _ = rx.unwatch(&path);
                }
            }
            self.files.reset_reader(&path);
        }

        for path in orphaned_readerless_paths {
            if !self.orphans.contains(&path) {
                ct_show_error!("directory containing watched file was removed");
                ct_show_error!(
                    "{} cannot be used, reverting to polling",
                    text::TAIL_BACKEND
                );
                self.use_polling = true;
                self.orphans.push(path.clone());
                if let Some(rx) = self.watcher_rx.as_mut() {
                    let _ = rx.unwatch(&path);
                }
            }
        }

        for (path, md) in unchanged_paths {
            self.files.update_metadata(&path, Some(md));
        }

        for path in steady_reader_paths {
            read_some = self.files.tail_file(&path, verbose)? || read_some;
        }

        for (path, md) in replaced_paths {
            let display_name = self.files.get(&path).display_name.clone();
            ct_show_error!(
                "{} has been replaced;  following new file",
                display_name.quote()
            );
            self.files.update_metadata(&path, Some(md));
            self.files.update_reader(&path)?;
            read_some = self.files.tail_file(&path, verbose)? || read_some;
        }

        for (path, md) in truncated_paths {
            let display_name = self.files.get(&path).display_name.clone();
            ct_show_error!("{}: file truncated", display_name);
            self.files.update_metadata(&path, Some(md));
            self.files.update_reader(&path)?;
            read_some = self.files.tail_file(&path, verbose)? || read_some;
        }

        for new_path in newly_appeared_paths {
            if let Ok(md) = new_path.metadata() {
                if md.is_tailable() {
                    let same_file_reappeared = self
                        .files
                        .get(&new_path)
                        .metadata
                        .as_ref()
                        .is_some_and(|old_md| old_md.file_id_eq(&md));

                    if same_file_reappeared {
                        self.files.update_metadata(&new_path, Some(md));
                        self.files.update_reader_at_end(&new_path)?;
                        if let Some(rx) = self.watcher_rx.as_mut() {
                            let _ = rx.watch_with_parent(&new_path);
                        }
                        continue;
                    }

                    let pd = self.files.get(&new_path);
                    ct_show_error!(
                        "{} has appeared;  following new file",
                        pd.display_name.quote()
                    );
                    self.files.update_metadata(&new_path, Some(md));
                    self.files.update_reader(&new_path)?;
                    read_some = self.files.tail_file(&new_path, verbose)? || read_some;
                    if let Some(rx) = self.watcher_rx.as_mut() {
                        let _ = rx.watch_with_parent(&new_path);
                    }
                }
            }
        }

        Ok(read_some)
    }
}

#[allow(clippy::cognitive_complexity)]
pub fn follow(mut observer: Observer, options: &TailOptions) -> CTResult<()> {
    if observer.files.no_files_remaining(options) && !observer.files.only_stdin_remaining() {
        return Err(CtSimpleError::new(
            1,
            text::TAIL_NO_FILES_REMAINING.to_string(),
        ));
    }

    let mut process = platform::ProcessChecker::new(observer.pid);

    let mut timeout_counter = 0;

    // main follow loop
    loop {
        // 第一时间探测标准 I/O 的生死状态
        let (stdin_dead, stdout_dead) = check_io_health();

        // 如果输出管道断裂（下游退出或被关闭），立即停止工作，防止变成僵尸进程
        if stdout_dead {
            break;
        }

        // 如果输入管道枯竭，将标准输入从监控名单中永久除名
        if stdin_dead {
            let mut dead_paths = vec![];
            for path in observer.files.keys() {
                if path.to_string_lossy() == "-" || path.to_string_lossy() == "/dev/stdin" {
                    dead_paths.push(path.clone());
                }
            }
            for p in dead_paths {
                observer.files.remove(&p);
            }
        }

        // 如果剔除死管道后，已经没有任何合法文件需要追踪了，优雅退出
        if observer.files.no_files_remaining(options) {
            break;
        }

        let mut _read_some = false;

        // If `--pid=p`, tail checks whether process p
        // is alive at least every `--sleep-interval=N` seconds
        if options.follow.is_some() && observer.pid != 0 && process.is_dead() {
            // p is dead, tail will also terminate
            break;
        }

        _read_some = observer.reconcile_follow_name_retry(options.verbose)? || _read_some;

        // With  -f, sleep for approximately N seconds (default 1.0) between iterations;
        // We wake up if Notify sends an Event or if we wait more than `sleep_sec`.
        let rx_result = observer
            .watcher_rx
            .as_mut()
            .unwrap()
            .receiver
            .recv_timeout(options.sleep_sec);

        if rx_result.is_ok() {
            timeout_counter = 0;
        }

        let mut paths = vec![]; // Paths worth checking for new content to print
        let mut timed_out = false;
        match rx_result {
            Ok(Ok(event)) => {
                if !event.paths.is_empty() {
                    // Handle Event if it is about a tracked path, including
                    // rename-to events that must be paired via the tracker id.
                    paths = observer.handle_event(&event, options)?;
                }
            }
            Ok(Err(notify::Error {
                kind: notify::ErrorKind::Io(ref e),
                paths: ref err_paths,
            })) if e.kind() == std::io::ErrorKind::NotFound => {
                if let Some(event_path) = err_paths.first() {
                    if observer.files.contains_key(event_path) {
                        let _ = observer
                            .watcher_rx
                            .as_mut()
                            .unwrap()
                            .watcher
                            .unwatch(event_path);
                    }
                }
            }
            Ok(Err(notify::Error {
                kind: notify::ErrorKind::MaxFilesWatch,
                ..
            })) => {
                return Err(CtSimpleError::new(
                    1,
                    format!("{} resources exhausted", text::TAIL_BACKEND),
                ));
            }
            Ok(Err(e)) => return Err(CtSimpleError::new(1, format!("NotifyError: {e}"))),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                timeout_counter += 1;
                timed_out = true;
            }
            Err(e) => return Err(CtSimpleError::new(1, format!("RecvTimeoutError: {e}"))),
        }

        if timed_out && observer.follow_descriptor() && options.follow.is_some() {
            paths = observer
                .files
                .keys()
                .filter_map(|path| {
                    observer
                        .files
                        .get(path)
                        .metadata
                        .as_ref()
                        .filter(|metadata| metadata.file_type().is_file())
                        .map(|_| path.clone())
                })
                .collect::<Vec<_>>();
        }

        if observer.use_polling && options.follow.is_some() {
            paths = observer.files.keys().cloned().collect::<Vec<_>>();
        }

        // main print loop
        for path in &paths {
            _read_some = observer.files.tail_file(path, options.verbose)?;
        }

        if timeout_counter == options.max_unchanged_stats {
            // (TODO preserved)
        }
    }
    Ok(())
}

#[cfg(unix)]
fn check_io_health() -> (bool, bool) {
    let mut stdin_dead = false;
    let mut stdout_dead = false;
    unsafe {
        let mut pfds = [
            libc::pollfd {
                fd: libc::STDIN_FILENO,
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: libc::STDOUT_FILENO,
                events: libc::POLLOUT,
                revents: 0,
            },
        ];
        if libc::poll(pfds.as_mut_ptr(), 2, 0) > 0 {
            // POLLHUP(挂断), POLLERR(错误), POLLNVAL(无效FD)
            if (pfds[0].revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL)) != 0 {
                stdin_dead = true;
            }
            if (pfds[1].revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL)) != 0 {
                stdout_dead = true;
            }
        }
    }
    (stdin_dead, stdout_dead)
}

#[cfg(not(unix))]
fn check_io_health() -> (bool, bool) {
    (false, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{ModifyKind, RemoveKind, RenameMode};
    use notify::{Event, EventKind};
    use std::fs::{self, File, OpenOptions};
    use std::io::{BufReader, Write};
    #[cfg(unix)]
    use std::os::unix::fs::symlink;
    use tempfile::tempdir;

    fn descriptor_options() -> TailOptions {
        TailOptions {
            follow: Some(TailFollowMode::Descriptor),
            ..TailOptions::default()
        }
    }

    #[test]
    fn split_rename_to_uses_pending_source_path() {
        let options = descriptor_options();
        let mut observer = Observer::from(&options);
        let temp_dir = tempdir().unwrap();
        let old_path = temp_dir.path().join("a");
        fs::write(&old_path, b"x\n").unwrap();
        observer
            .add_path(
                &old_path,
                "a",
                Some(Box::new(BufReader::new(File::open(&old_path).unwrap()))),
                false,
            )
            .unwrap();
        observer.pending_renames.insert(7, old_path.clone());

        let new_path = temp_dir.path().join("b");
        fs::rename(&old_path, &new_path).unwrap();

        let event = Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::To)))
            .set_tracker(7)
            .add_path(new_path);

        assert_eq!(observer.tracked_path_for_event(&event), Some(old_path));
    }

    #[test]
    fn split_rename_events_move_descriptor_tracking_to_new_path() {
        let options = descriptor_options();
        let mut observer = Observer::from(&options);
        let temp_dir = tempdir().unwrap();
        let old_path = temp_dir.path().join("a");
        let new_path = temp_dir.path().join("b");

        fs::write(&old_path, b"x\n").unwrap();
        observer
            .add_path(
                &old_path,
                "a",
                Some(Box::new(BufReader::new(File::open(&old_path).unwrap()))),
                false,
            )
            .unwrap();

        let from_event = Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::From)))
            .set_tracker(9)
            .add_path(old_path.clone());
        assert!(
            observer
                .handle_event(&from_event, &options)
                .unwrap()
                .is_empty()
        );
        assert_eq!(observer.pending_renames.get(&9), Some(&old_path));

        fs::rename(&old_path, &new_path).unwrap();

        let to_event = Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::To)))
            .set_tracker(9)
            .add_path(new_path.clone());
        assert_eq!(
            observer.handle_event(&to_event, &options).unwrap(),
            vec![new_path.clone()]
        );
        assert!(observer.files.contains_key(&new_path));
        assert!(!observer.files.contains_key(&old_path));
        assert!(observer.pending_renames.is_empty());
    }

    #[test]
    fn directory_create_event_is_not_mapped_to_arbitrary_sibling_file() {
        let options = TailOptions {
            follow: Some(TailFollowMode::Name),
            retry: true,
            ..TailOptions::default()
        };
        let mut observer = Observer::from(&options);
        let temp_dir = tempdir().unwrap();
        let watched_dir = temp_dir.path().join("dir");
        let a = watched_dir.join("a");
        let b = watched_dir.join("b");

        fs::create_dir_all(&watched_dir).unwrap();
        fs::write(&a, b"a\n").unwrap();
        fs::write(&b, b"b\n").unwrap();
        observer
            .add_path(
                &a,
                "a",
                Some(Box::new(BufReader::new(File::open(&a).unwrap()))),
                false,
            )
            .unwrap();
        observer
            .add_path(
                &b,
                "b",
                Some(Box::new(BufReader::new(File::open(&b).unwrap()))),
                false,
            )
            .unwrap();

        let event =
            Event::new(EventKind::Create(notify::event::CreateKind::Folder)).add_path(watched_dir);
        assert_eq!(observer.tracked_path_for_event(&event), None);
    }

    #[cfg(unix)]
    #[test]
    fn reconcile_follow_name_retry_detects_symlink_target_switch() {
        let options = TailOptions {
            follow: Some(TailFollowMode::Name),
            retry: true,
            ..TailOptions::default()
        };
        let mut observer = Observer::from(&options);
        let temp_dir = tempdir().unwrap();
        let link_path = temp_dir.path().join("symlink");
        let target1 = temp_dir.path().join("target1");
        let target2 = temp_dir.path().join("target2");

        fs::write(&target1, b"X1\n").unwrap();
        symlink(&target1, &link_path).unwrap();
        observer
            .add_path(
                &link_path,
                "symlink",
                Some(Box::new(BufReader::new(File::open(&link_path).unwrap()))),
                false,
            )
            .unwrap();

        assert!(observer.files.tail_file(&link_path, false).unwrap());

        fs::remove_file(&link_path).unwrap();
        symlink(&target2, &link_path).unwrap();
        assert!(!observer.reconcile_follow_name_retry(false).unwrap());
        assert!(observer.files.get(&link_path).reader.is_none());

        fs::write(&target2, b"X2\n").unwrap();
        assert!(observer.reconcile_follow_name_retry(false).unwrap());
        let watched_md = observer.files.get(&link_path).metadata.as_ref().unwrap();
        assert!(watched_md.file_id_eq(&target2.metadata().unwrap()));
    }

    #[cfg(unix)]
    #[test]
    fn reconcile_follow_name_retry_reads_data_after_empty_appearance() {
        let options = TailOptions {
            follow: Some(TailFollowMode::Name),
            retry: true,
            ..TailOptions::default()
        };
        let mut observer = Observer::from(&options);
        let temp_dir = tempdir().unwrap();
        let link_path = temp_dir.path().join("symlink");
        let target = temp_dir.path().join("target");

        symlink(&target, &link_path).unwrap();
        observer.add_bad_path(&link_path, "symlink", false).unwrap();

        fs::write(&target, b"").unwrap();
        assert!(!observer.reconcile_follow_name_retry(false).unwrap());

        let mut target_file = OpenOptions::new().append(true).open(&target).unwrap();
        writeln!(target_file, "X").unwrap();
        assert!(observer.reconcile_follow_name_retry(false).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn reconcile_follow_name_retry_skips_reread_when_same_file_reappears() {
        let options = TailOptions {
            follow: Some(TailFollowMode::Name),
            retry: true,
            ..TailOptions::default()
        };
        let mut observer = Observer::from(&options);
        let temp_dir = tempdir().unwrap();
        let link_path = temp_dir.path().join("symlink");
        let target = temp_dir.path().join("target");

        fs::write(&target, b"X1\n").unwrap();
        symlink(&target, &link_path).unwrap();
        observer
            .add_path(
                &link_path,
                "symlink",
                Some(Box::new(BufReader::new(File::open(&link_path).unwrap()))),
                false,
            )
            .unwrap();
        assert!(observer.files.tail_file(&link_path, false).unwrap());

        fs::remove_file(&link_path).unwrap();
        assert!(!observer.reconcile_follow_name_retry(false).unwrap());
        symlink(&target, &link_path).unwrap();
        assert!(!observer.reconcile_follow_name_retry(false).unwrap());

        let mut target_file = OpenOptions::new().append(true).open(&target).unwrap();
        writeln!(target_file, "X2").unwrap();
        assert!(observer.reconcile_follow_name_retry(false).unwrap());
    }

    #[test]
    fn remove_dir_event_switches_to_polling_for_orphaned_tracked_file() {
        let options = TailOptions {
            follow: Some(TailFollowMode::Name),
            retry: true,
            ..TailOptions::default()
        };
        let mut observer = Observer::from(&options);
        let temp_dir = tempdir().unwrap();
        let watched_dir = temp_dir.path().join("dir");
        let watched_file = watched_dir.join("file");

        fs::create_dir_all(&watched_dir).unwrap();
        fs::write(&watched_file, b"X\n").unwrap();
        observer
            .add_path(
                &watched_file,
                "dir/file",
                Some(Box::new(BufReader::new(File::open(&watched_file).unwrap()))),
                false,
            )
            .unwrap();

        fs::remove_dir_all(&watched_dir).unwrap();

        let event = Event::new(EventKind::Remove(RemoveKind::Folder)).add_path(watched_dir);
        let _ = observer.handle_event(&event, &options).unwrap();

        assert!(observer.use_polling);
        assert!(observer.orphans.contains(&watched_file));
        assert!(observer.files.get(&watched_file).reader.is_none());
    }

    #[test]
    fn reconcile_marks_orphaned_readerless_path_as_polling() {
        let options = TailOptions {
            follow: Some(TailFollowMode::Name),
            retry: true,
            ..TailOptions::default()
        };
        let mut observer = Observer::from(&options);
        let temp_dir = tempdir().unwrap();
        let watched_dir = temp_dir.path().join("dir");
        let watched_file = watched_dir.join("file");

        fs::create_dir_all(&watched_dir).unwrap();
        fs::write(&watched_file, b"X\n").unwrap();
        observer
            .add_path(
                &watched_file,
                "dir/file",
                Some(Box::new(BufReader::new(File::open(&watched_file).unwrap()))),
                false,
            )
            .unwrap();
        observer.files.reset_reader(&watched_file);

        fs::remove_dir_all(&watched_dir).unwrap();

        assert!(!observer.reconcile_follow_name_retry(false).unwrap());
        assert!(observer.use_polling);
        assert!(observer.orphans.contains(&watched_file));
    }

    #[test]
    fn remove_file_event_catches_following_parent_removal() {
        let options = TailOptions {
            follow: Some(TailFollowMode::Name),
            retry: true,
            ..TailOptions::default()
        };
        let mut observer = Observer::from(&options);
        let temp_dir = tempdir().unwrap();
        let watched_dir = temp_dir.path().join("dir");
        let watched_file = watched_dir.join("file");

        fs::create_dir_all(&watched_dir).unwrap();
        fs::write(&watched_file, b"X\n").unwrap();
        observer
            .add_path(
                &watched_file,
                "dir/file",
                Some(Box::new(BufReader::new(File::open(&watched_file).unwrap()))),
                false,
            )
            .unwrap();

        fs::remove_file(&watched_file).unwrap();
        let dir_for_thread = watched_dir.clone();
        let remover = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            let _ = fs::remove_dir_all(dir_for_thread);
        });

        let event = Event::new(EventKind::Remove(RemoveKind::File)).add_path(watched_file.clone());
        let _ = observer.handle_event(&event, &options).unwrap();
        remover.join().unwrap();

        assert!(observer.use_polling);
        assert!(observer.orphans.contains(&watched_file));
        assert!(observer.files.get(&watched_file).reader.is_none());
    }
}
