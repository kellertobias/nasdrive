use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, RwLock,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

use nasfiles_core::models::{AuthUser, FileEntry, Root};
use serde::{Deserialize, Serialize};

use crate::config::AppConfig;
use crate::fs::{roots, sanitize_header_filename};
use crate::state::AppState;
use crate::thumb::kind;

#[derive(Clone)]
pub struct SearchService {
    config: Arc<AppConfig>,
    index: Arc<RwLock<SearchIndex>>,
    refresh_running: Arc<AtomicBool>,
}

#[derive(Default)]
struct SearchIndex {
    docs: HashMap<String, SearchDoc>,
    ready: bool,
}

#[derive(Clone, Debug)]
struct SearchDoc {
    root_scope: String,
    root_key: String,
    relative_path: String,
    parent_path: String,
    name: String,
    name_norm: String,
    path_norm: String,
    search_text: String,
    size: u64,
    modified_at: i64,
    is_dir: bool,
    mime_type: Option<String>,
    has_thumbnail: bool,
}

#[derive(Serialize)]
pub struct SearchResponse {
    pub query: String,
    pub results: Vec<SearchResult>,
    pub live_complete: bool,
    pub index_ready: bool,
}

#[derive(Serialize)]
pub struct SearchResult {
    pub root: String,
    pub root_display_name: String,
    pub path: String,
    pub parent_path: String,
    pub entry: FileEntry,
    pub source: SearchSource,
    pub score: i64,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchSource {
    Index,
    Live,
}

struct LiveRoot {
    root_key: String,
    root_scope: String,
    root_path: PathBuf,
}

struct ScanOptions<'a> {
    thumbnails_enabled: bool,
    budget: usize,
    deadline: Option<Instant>,
    terms: Option<&'a [String]>,
    query_norm: Option<&'a str>,
}

struct LiveScan {
    docs: Vec<(SearchDoc, i64)>,
    visited_docs: Vec<SearchDoc>,
    complete: bool,
}

#[derive(Deserialize)]
struct DiskStatePayload {
    #[serde(default)]
    pools: Vec<PoolDiskState>,
}

#[derive(Deserialize)]
struct PoolDiskState {
    name: String,
    #[serde(default)]
    hdds_spinning: bool,
}

impl SearchService {
    pub fn new(config: Arc<AppConfig>) -> Self {
        Self {
            config,
            index: Arc::new(RwLock::new(SearchIndex::default())),
            refresh_running: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn spawn_refresh_loop(&self) {
        if self.config.search_reindex_interval_secs == 0
            && self.config.search_full_reindex_interval_secs == 0
        {
            tracing::info!("background search reindexing disabled");
            return;
        }

        let service = self.clone();
        tokio::spawn(async move {
            service.refresh_common_roots(false).await;
            let mut regular = (service.config.search_reindex_interval_secs > 0).then(|| {
                tokio::time::interval(Duration::from_secs(
                    service.config.search_reindex_interval_secs,
                ))
            });
            let mut full = (service.config.search_full_reindex_interval_secs > 0).then(|| {
                tokio::time::interval(Duration::from_secs(
                    service.config.search_full_reindex_interval_secs,
                ))
            });
            if let Some(interval) = regular.as_mut() {
                interval.tick().await;
            }
            if let Some(interval) = full.as_mut() {
                interval.tick().await;
            }
            loop {
                tokio::select! {
                    _ = async { regular.as_mut().unwrap().tick().await }, if regular.is_some() => {
                        service.refresh_common_roots(false).await;
                    }
                    _ = async { full.as_mut().unwrap().tick().await }, if full.is_some() => {
                        tracing::info!("starting scheduled full search reindex");
                        service.refresh_common_roots(true).await;
                    }
                }
            }
        });
    }

    pub fn schedule_user_refresh(&self, state: AppState, user: AuthUser) {
        let service = self.clone();
        tokio::spawn(async move {
            let _ = service.refresh_visible_roots(&state, &user).await;
        });
    }

    pub fn remove_paths_for_user(&self, user: &AuthUser, root_key: &str, paths: &[String]) {
        let scope = root_scope(root_key, user);
        let mut index = self.index.write().unwrap();
        for path in paths {
            remove_path_locked(&mut index.docs, &scope, path);
        }
    }

    pub async fn search(
        &self,
        state: &AppState,
        user: &AuthUser,
        raw_query: &str,
        requested_limit: Option<usize>,
    ) -> SearchResponse {
        let query = raw_query.trim();
        let query_norm = normalize(query);
        let terms = query_terms(query);
        let index_ready = self.index.read().unwrap().ready;
        if terms.is_empty() {
            return SearchResponse {
                query: query.to_string(),
                results: Vec::new(),
                live_complete: true,
                index_ready,
            };
        }

        let limit = requested_limit
            .unwrap_or(self.config.search_max_results)
            .clamp(1, self.config.search_max_results);
        let visible_roots = roots::visible_roots(&state.config, user);
        let root_by_key: HashMap<String, Root> = visible_roots
            .iter()
            .cloned()
            .map(|root| (root.key.clone(), root))
            .collect();
        let visible_scopes: HashSet<String> = visible_roots
            .iter()
            .map(|root| root_scope(&root.key, user))
            .collect();

        let mut candidates = self.index_candidates(&visible_scopes, &terms, &query_norm);
        candidates.truncate(limit.saturating_mul(4).max(limit));

        let mut results = Vec::with_capacity(limit);
        let mut seen = HashSet::new();
        for (doc, score) in candidates {
            if results.len() >= limit {
                break;
            }
            let key = result_key(&doc.root_scope, &doc.relative_path);
            if !seen.insert(key) {
                continue;
            }
            match self.verify_doc(state, user, &root_by_key, &doc, SearchSource::Index, score) {
                Some(result) => results.push(result),
                None => self.remove_doc(&doc.root_scope, &doc.relative_path),
            }
        }

        let live_roots = visible_roots
            .iter()
            .filter_map(|root| {
                roots::resolve_root(&state.config, user, &root.key, roots::RequiredCap::Read)
                    .ok()
                    .map(|root_path| LiveRoot {
                        root_key: root.key.clone(),
                        root_scope: root_scope(&root.key, user),
                        root_path,
                    })
            })
            .collect::<Vec<_>>();

        let live_scan = self.scan_live(live_roots, terms.clone(), query_norm).await;
        for doc in live_scan.visited_docs {
            self.insert_doc(doc);
        }
        for (doc, score) in live_scan.docs {
            if results.len() >= limit {
                break;
            }
            let key = result_key(&doc.root_scope, &doc.relative_path);
            if !seen.insert(key) {
                continue;
            }
            let root_display_name = root_by_key
                .get(&doc.root_key)
                .map(|root| root.display_name.clone())
                .unwrap_or_else(|| doc.root_key.clone());
            results.push(doc.into_result(root_display_name, SearchSource::Live, score));
        }

        results.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then_with(|| b.entry.modified_at.cmp(&a.entry.modified_at))
                .then_with(|| a.path.to_lowercase().cmp(&b.path.to_lowercase()))
        });
        results.truncate(limit);

        SearchResponse {
            query: query.to_string(),
            results,
            live_complete: live_scan.complete,
            index_ready,
        }
    }

    async fn refresh_common_roots(&self, include_sleeping_hdds: bool) {
        if self.refresh_running.swap(true, Ordering::Relaxed) {
            return;
        }

        let config = self.config.clone();
        let scan = tokio::task::spawn_blocking(move || {
            let spinning_hdd_pools = spinning_hdd_pools(
                config.search_disk_state_file.as_deref(),
                &config.search_hdd_pools,
            );
            tracing::info!(
                full = include_sleeping_hdds,
                spinning_hdd_pools = ?spinning_hdd_pools,
                "refreshing common search roots"
            );
            let mut docs_by_scope = HashMap::<String, Vec<SearchDoc>>::new();
            for (root_key, root_path) in &config.common_folders {
                if !include_sleeping_hdds
                    && root_hdd_pool(root_path, &config.search_hdd_pools)
                        .is_some_and(|pool| !spinning_hdd_pools.contains(pool))
                {
                    continue;
                }
                let root = LiveRoot {
                    root_key: root_key.clone(),
                    root_scope: root_key.clone(),
                    root_path: root_path.clone(),
                };
                let docs = scan_root(
                    &root,
                    ScanOptions {
                        thumbnails_enabled: !config.no_server_side_execution,
                        budget: usize::MAX,
                        deadline: None,
                        terms: None,
                        query_norm: None,
                    },
                )
                .visited_docs;
                docs_by_scope.insert(root.root_scope, docs);
            }
            docs_by_scope
        })
        .await;

        if let Ok(docs_by_scope) = scan {
            let mut index = self.index.write().unwrap();
            for scope in docs_by_scope.keys() {
                index.docs.retain(|_, doc| &doc.root_scope != scope);
            }
            for docs in docs_by_scope.into_values() {
                for doc in docs {
                    index.docs.insert(doc.key(), doc);
                }
            }
            index.ready = true;
        }

        self.refresh_running.store(false, Ordering::Relaxed);
    }

    async fn refresh_visible_roots(&self, state: &AppState, user: &AuthUser) -> Result<(), ()> {
        let roots = roots::visible_roots(&state.config, user)
            .into_iter()
            .filter_map(|root| {
                roots::resolve_root(&state.config, user, &root.key, roots::RequiredCap::Read)
                    .ok()
                    .map(|root_path| LiveRoot {
                        root_key: root.key.clone(),
                        root_scope: root_scope(&root.key, user),
                        root_path,
                    })
            })
            .collect::<Vec<_>>();
        let thumbnails_enabled = !state.config.no_server_side_execution;
        let scan = tokio::task::spawn_blocking(move || {
            roots
                .into_iter()
                .map(|root| {
                    let scope = root.root_scope.clone();
                    let docs = scan_root(
                        &root,
                        ScanOptions {
                            thumbnails_enabled,
                            budget: usize::MAX,
                            deadline: None,
                            terms: None,
                            query_norm: None,
                        },
                    )
                    .visited_docs;
                    (scope, docs)
                })
                .collect::<Vec<_>>()
        })
        .await
        .map_err(|_| ())?;

        let mut index = self.index.write().unwrap();
        for (scope, docs) in scan {
            index.docs.retain(|_, doc| doc.root_scope != scope);
            for doc in docs {
                index.docs.insert(doc.key(), doc);
            }
        }
        index.ready = true;
        Ok(())
    }

    async fn scan_live(
        &self,
        roots: Vec<LiveRoot>,
        terms: Vec<String>,
        query_norm: String,
    ) -> LiveScan {
        let thumbnails_enabled = !self.config.no_server_side_execution;
        let budget = self.config.search_live_entry_budget;
        let deadline =
            Instant::now() + Duration::from_millis(self.config.search_live_time_budget_ms);
        tokio::task::spawn_blocking(move || {
            let mut complete = true;
            let mut remaining_budget = budget;
            let mut matched = Vec::new();
            let mut visited_docs = Vec::new();
            for root in roots {
                if remaining_budget == 0 || Instant::now() >= deadline {
                    complete = false;
                    break;
                }
                let scan = scan_root(
                    &root,
                    ScanOptions {
                        thumbnails_enabled,
                        budget: remaining_budget,
                        deadline: Some(deadline),
                        terms: Some(&terms),
                        query_norm: Some(&query_norm),
                    },
                );
                remaining_budget = remaining_budget.saturating_sub(scan.visited_docs.len());
                complete &= scan.complete;
                matched.extend(scan.docs);
                visited_docs.extend(scan.visited_docs);
            }
            LiveScan {
                docs: matched,
                visited_docs,
                complete,
            }
        })
        .await
        .unwrap_or(LiveScan {
            docs: Vec::new(),
            visited_docs: Vec::new(),
            complete: false,
        })
    }

    fn index_candidates(
        &self,
        visible_scopes: &HashSet<String>,
        terms: &[String],
        query_norm: &str,
    ) -> Vec<(SearchDoc, i64)> {
        let index = self.index.read().unwrap();
        let mut scored = index
            .docs
            .values()
            .filter(|doc| visible_scopes.contains(&doc.root_scope))
            .filter_map(|doc| score_doc(doc, terms, query_norm).map(|score| (doc.clone(), score)))
            .collect::<Vec<_>>();
        scored.sort_by(|(a, a_score), (b, b_score)| {
            b_score
                .cmp(a_score)
                .then_with(|| b.modified_at.cmp(&a.modified_at))
                .then_with(|| a.relative_path.cmp(&b.relative_path))
        });
        scored
    }

    fn verify_doc(
        &self,
        state: &AppState,
        user: &AuthUser,
        root_by_key: &HashMap<String, Root>,
        doc: &SearchDoc,
        source: SearchSource,
        score: i64,
    ) -> Option<SearchResult> {
        let root = root_by_key.get(&doc.root_key)?;
        let root_path =
            roots::resolve_root(&state.config, user, &doc.root_key, roots::RequiredCap::Read)
                .ok()?;
        let resolved = nasfiles_core::safe_path::resolve(&root_path, &doc.relative_path).ok()?;
        let fresh = doc_from_path(
            &doc.root_key,
            &doc.root_scope,
            &doc.relative_path,
            &resolved,
            !state.config.no_server_side_execution,
        )?;
        Some(fresh.into_result(root.display_name.clone(), source, score))
    }

    fn insert_doc(&self, doc: SearchDoc) {
        self.index.write().unwrap().docs.insert(doc.key(), doc);
    }

    fn remove_doc(&self, root_scope: &str, relative_path: &str) {
        self.index
            .write()
            .unwrap()
            .docs
            .remove(&result_key(root_scope, relative_path));
    }
}

fn root_hdd_pool<'a>(path: &Path, hdd_pools: &'a HashSet<String>) -> Option<&'a str> {
    let mut components = path.components();
    let _root = components.next()?;
    if components.next()?.as_os_str() != "mnt" {
        return None;
    }
    let pool = components.next()?.as_os_str().to_str()?;
    hdd_pools.get(pool).map(String::as_str)
}

fn spinning_hdd_pools(
    disk_state_file: Option<&Path>,
    hdd_pools: &HashSet<String>,
) -> HashSet<String> {
    let Some(path) = disk_state_file else {
        return hdd_pools.clone();
    };
    let fresh = std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| modified.elapsed().ok())
        .is_some_and(|age| age <= Duration::from_secs(120));
    if !fresh {
        tracing::warn!(path = %path.display(), "disk-state telemetry missing or stale; skipping HDD search roots");
        return HashSet::new();
    }
    let Ok(payload) = std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<DiskStatePayload>(&bytes).ok())
        .ok_or(())
    else {
        tracing::warn!(path = %path.display(), "invalid disk-state telemetry; skipping HDD search roots");
        return HashSet::new();
    };
    payload
        .pools
        .into_iter()
        .filter(|pool| pool.hdds_spinning && hdd_pools.contains(&pool.name))
        .map(|pool| pool.name)
        .collect()
}

impl SearchDoc {
    fn key(&self) -> String {
        result_key(&self.root_scope, &self.relative_path)
    }

    fn into_result(
        self,
        root_display_name: String,
        source: SearchSource,
        score: i64,
    ) -> SearchResult {
        SearchResult {
            root: self.root_key,
            root_display_name,
            path: self.relative_path.clone(),
            parent_path: self.parent_path,
            entry: FileEntry {
                name: self.name,
                size: self.size,
                modified_at: self.modified_at,
                is_dir: self.is_dir,
                mime_type: self.mime_type,
                has_thumbnail: self.has_thumbnail,
                media_info: None,
                image_info: None,
                item_count: None,
            },
            source,
            score,
        }
    }
}

fn scan_root(root: &LiveRoot, options: ScanOptions<'_>) -> LiveScan {
    let mut complete = true;
    let mut visited = 0usize;
    let mut matched = Vec::new();
    let mut visited_docs = Vec::new();
    let mut pending = vec![(root.root_path.clone(), String::new())];

    while let Some((dir, parent_rel)) = pending.pop() {
        if visited >= options.budget
            || options
                .deadline
                .is_some_and(|deadline| Instant::now() >= deadline)
        {
            complete = false;
            break;
        }

        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };

        for entry in entries.flatten() {
            if visited >= options.budget
                || options
                    .deadline
                    .is_some_and(|deadline| Instant::now() >= deadline)
            {
                complete = false;
                break;
            }

            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }

            let relative_path = join_rel(&parent_rel, &name);
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            let Some(doc) = doc_from_metadata(
                &root.root_key,
                &root.root_scope,
                relative_path,
                entry.path(),
                metadata,
                options.thumbnails_enabled,
            ) else {
                continue;
            };
            visited += 1;

            if let (Some(terms), Some(query_norm)) = (options.terms, options.query_norm)
                && let Some(score) = score_doc(&doc, terms, query_norm)
            {
                matched.push((doc.clone(), score));
            }

            let should_descend = doc.is_dir
                && entry
                    .file_type()
                    .map(|file_type| !file_type.is_symlink())
                    .unwrap_or(false);
            if should_descend {
                pending.push((entry.path(), doc.relative_path.clone()));
            }
            visited_docs.push(doc);
        }
    }

    LiveScan {
        docs: matched,
        visited_docs,
        complete,
    }
}

fn doc_from_path(
    root_key: &str,
    root_scope: &str,
    relative_path: &str,
    path: &Path,
    thumbnails_enabled: bool,
) -> Option<SearchDoc> {
    if hidden_path(relative_path) {
        return None;
    }
    let metadata = std::fs::metadata(path).ok()?;
    doc_from_metadata(
        root_key,
        root_scope,
        relative_path.to_string(),
        path.to_path_buf(),
        metadata,
        thumbnails_enabled,
    )
}

fn doc_from_metadata(
    root_key: &str,
    root_scope: &str,
    relative_path: String,
    path: PathBuf,
    metadata: std::fs::Metadata,
    thumbnails_enabled: bool,
) -> Option<SearchDoc> {
    if !metadata.is_dir() && !metadata.is_file() {
        return None;
    }
    if hidden_path(&relative_path) {
        return None;
    }

    let name = path.file_name()?.to_string_lossy().to_string();
    let is_dir = metadata.is_dir();
    let size = if is_dir { 0 } else { metadata.len() };
    let modified_at = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let mime_type = if is_dir {
        None
    } else {
        mime_guess::from_path(&path).first().map(|m| m.to_string())
    };
    let has_thumbnail = !is_dir && kind::supports_thumbnail_path(&path, thumbnails_enabled);
    let parent_path = parent_path(&relative_path);
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_string();
    let kind = if is_dir { "folder directory" } else { "file" };
    let name_norm = normalize(&name);
    let path_norm = normalize(&relative_path);
    let mime_norm = normalize(mime_type.as_deref().unwrap_or(""));
    let ext_norm = normalize(&extension);
    let search_text = format!("{name_norm} {path_norm} {mime_norm} {ext_norm} {kind}");

    Some(SearchDoc {
        root_scope: root_scope.to_string(),
        root_key: root_key.to_string(),
        relative_path,
        parent_path,
        name,
        name_norm,
        path_norm,
        search_text,
        size,
        modified_at,
        is_dir,
        mime_type,
        has_thumbnail,
    })
}

fn score_doc(doc: &SearchDoc, terms: &[String], query_norm: &str) -> Option<i64> {
    if !terms.iter().all(|term| doc.search_text.contains(term)) {
        return None;
    }

    let mut score = 100;
    if doc.name_norm == query_norm {
        score += 10_000;
    }
    if doc.name_norm.starts_with(query_norm) {
        score += 7_000;
    }
    if doc.name_norm.contains(query_norm) {
        score += 5_000;
    }
    if doc.path_norm.contains(query_norm) {
        score += 2_000;
    }
    for term in terms {
        if doc.name_norm == *term {
            score += 1_500;
        } else if doc.name_norm.starts_with(term) {
            score += 900;
        } else if doc.name_norm.contains(term) {
            score += 600;
        } else if doc.path_norm.contains(term) {
            score += 300;
        }
    }
    if doc.is_dir {
        score += 50;
    }
    Some(score)
}

fn root_scope(root_key: &str, user: &AuthUser) -> String {
    if root_key == "~" {
        format!("~:{}", sanitize_header_filename(&user.safe_username()))
    } else {
        root_key.to_string()
    }
}

fn remove_path_locked(
    docs: &mut HashMap<String, SearchDoc>,
    root_scope: &str,
    relative_path: &str,
) {
    let prefix = format!("{}/", relative_path.trim_end_matches('/'));
    docs.retain(|_, doc| {
        doc.root_scope != root_scope
            || (doc.relative_path != relative_path && !doc.relative_path.starts_with(&prefix))
    });
}

fn result_key(root_scope: &str, relative_path: &str) -> String {
    format!("{root_scope}\0{relative_path}")
}

fn join_rel(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_string()
    } else {
        format!("{parent}/{name}")
    }
}

fn parent_path(path: &str) -> String {
    Path::new(path)
        .parent()
        .and_then(|parent| parent.to_str())
        .filter(|parent| *parent != ".")
        .unwrap_or("")
        .to_string()
}

fn hidden_path(path: &str) -> bool {
    path.split('/').any(|part| part.starts_with('.'))
}

fn query_terms(query: &str) -> Vec<String> {
    normalize(query)
        .split_whitespace()
        .filter(|term| !term.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn normalize(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn scores_name_matches_above_path_matches() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("quarterly-report.pdf");
        fs::write(&path, b"pdf").unwrap();
        let doc =
            doc_from_path("docs", "docs", "Reports/quarterly-report.pdf", &path, true).unwrap();

        let score = score_doc(&doc, &query_terms("quarterly"), &normalize("quarterly")).unwrap();
        assert!(score > 5_000);
    }

    #[test]
    fn skips_hidden_paths() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".secret.txt");
        fs::write(&path, b"secret").unwrap();

        assert!(doc_from_path("docs", "docs", ".secret.txt", &path, true).is_none());
    }

    #[test]
    fn removes_subtree_paths() {
        let mut docs = HashMap::new();
        for rel in ["Folder", "Folder/a.txt", "Folder/Nested/b.txt", "Other.txt"] {
            docs.insert(
                result_key("docs", rel),
                SearchDoc {
                    root_scope: "docs".into(),
                    root_key: "docs".into(),
                    relative_path: rel.into(),
                    parent_path: parent_path(rel),
                    name: rel.into(),
                    name_norm: normalize(rel),
                    path_norm: normalize(rel),
                    search_text: normalize(rel),
                    size: 0,
                    modified_at: 0,
                    is_dir: false,
                    mime_type: None,
                    has_thumbnail: false,
                },
            );
        }

        remove_path_locked(&mut docs, "docs", "Folder");

        assert!(!docs.contains_key(&result_key("docs", "Folder/a.txt")));
        assert!(docs.contains_key(&result_key("docs", "Other.txt")));
    }

    #[test]
    fn identifies_configured_hdd_pool_from_mount_path() {
        let pools = HashSet::from(["archive-pool".to_string()]);
        assert_eq!(
            root_hdd_pool(Path::new("/mnt/archive-pool/Video"), &pools),
            Some("archive-pool")
        );
        assert_eq!(
            root_hdd_pool(Path::new("/mnt/data-pool/Media"), &pools),
            None
        );
    }

    #[test]
    fn reads_only_configured_spinning_pools_from_fresh_telemetry() {
        let dir = tempfile::tempdir().unwrap();
        let state_file = dir.path().join("pools.json");
        fs::write(
            &state_file,
            r#"{"pools":[{"name":"archive-pool","hdds_spinning":true},{"name":"bulk-pool","hdds_spinning":false},{"name":"other-pool","hdds_spinning":true}]}"#,
        )
        .unwrap();
        let configured = HashSet::from(["archive-pool".to_string(), "bulk-pool".to_string()]);

        assert_eq!(
            spinning_hdd_pools(Some(&state_file), &configured),
            HashSet::from(["archive-pool".to_string()])
        );
    }
}
