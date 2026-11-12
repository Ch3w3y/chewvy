//! Bounded async artwork loading for the browse rows.
//!
//! [`WORKERS`] worker threads fetch `/photo/:/transcode` answers through
//! [`LiveClient::raw_get`] and decode them off the UI thread; [`Artwork::drain`]
//! uploads finished frames as egui textures on the UI thread and
//! [`Artwork::poster`] draws a poster (or the card placeholder while one is
//! missing). Requests are built by [`plex_core::library::photo_transcode_path`],
//! which rejects an artwork value naming the token header, so a token can reach
//! neither the URL nor a disk-cache key. Textures are held in an LRU capped at
//! [`MAX_TEXTURES`] and [`TEXTURE_BUDGET`] bytes of GPU memory; sizes are asked
//! in points and fetched at the display's pixel density; finished answers are cached under
//! `$XDG_CACHE_HOME/chewvy/art` named by SHA-256 over the artwork
//! path plus the requested size.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use egui::{ColorImage, Context, Response, Sense, TextureHandle, TextureOptions, Ui, Vec2};

use plex_core::library::photo_transcode_path;

use crate::sources::browse;

use crate::shell::live::LiveClient;
use crate::ui::art_cache::{env_dir, DiskCache};
use crate::ui::theme;

/// Poster size requested from `/photo/:/transcode` and drawn left of a row.
pub const POSTER: [u16; 2] = [60, 90];
/// Textures one [`Artwork`] holds before the least-recently-used is freed.
pub const MAX_TEXTURES: usize = 400;
/// GPU bytes the held textures may take, mip levels included: a full 4K screen
/// of artwork is ~45 MiB, so this keeps several screens without thrashing.
pub const TEXTURE_BUDGET: usize = 160 * 1024 * 1024;
/// Fetched widths snap up to this many pixels, so a window resize does not ask
/// for a fresh copy of every image.
const SIZE_STEP: f32 = 64.0;
/// The widest image ever asked for.
const MAX_FETCH_WIDTH: f32 = 3840.0;
/// Beyond this ratio of the requested size to the answer size the answer is
/// smaller than the request, so it is scaled up on a worker thread; at exactly
/// the requested size there is nothing to do.
const UPSCALE_BELOW: f32 = 1.0;
/// Artwork sampling: linear with mipmaps, so a poster drawn smaller than its
/// texture stays smooth instead of shimmering.
const ART_TEXTURE: TextureOptions = TextureOptions {
    mipmap_mode: Some(egui::TextureFilter::Linear),
    ..TextureOptions::LINEAR
};
/// Fetch/decode threads.
pub const WORKERS: usize = 4;
/// Requests queued ahead of the workers; the oldest is dropped when full.
const QUEUE_CAP: usize = 256;
/// Longest a worker waits for queued work before checking for shutdown.
const QUEUE_WAIT: Duration = Duration::from_millis(100);
/// First cooldown after a failed artwork attempt, doubled per attempt.
const RETRY_BASE: Duration = Duration::from_secs(30);
/// Longest cooldown for one key, whatever the attempt count.
const RETRY_CAP: Duration = Duration::from_secs(30 * 60);
/// Attempts for one key before it is left a placeholder until the next
/// [`Artwork::attach`]; a poster the server keeps rejecting is not retried.
const MAX_ATTEMPTS: u32 = 4;
/// Largest `/photo/:/transcode` answer accepted for decoding.
const MAX_ENCODED_BYTES: usize = 8 * 1024 * 1024;
/// Largest image side accepted: posters and backdrops are far smaller.
const MAX_DECODE_DIM: u32 = 4096;
/// Largest decoder allocation, bounding `width * height * 4` pixel buffers.
const MAX_DECODE_ALLOC: u64 = 64 * 1024 * 1024;
/// An answer is scaled down on the CPU only when the drawn size is under this
/// share of it; closer than that, the GPU's own filtering is as sharp.
const DOWNSCALE_FROM: f32 = 0.85;
/// Cache keys whose answer size has been logged; the log is a debug aid for
/// spotting small source art, not a history to keep forever.
const LOGGED_KEYS_CAP: usize = 2048;

/// SHA-256 hex of `<request path>\0<width>x<height>`: the disk-cache file name,
/// stable across runs and distinct per requested size.
///
/// Takes a path [`photo_transcode_path`] produced, never a token and never a
/// value the server named unchecked.
#[must_use]
pub fn cache_key(request_path: &str, width: u32, height: u32) -> String {
    let input = format!("{request_path}\u{0}{width}x{height}");
    ring::digest::digest(&ring::digest::SHA256, input.as_bytes())
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The pixel size fetched for art drawn `width`×`height` points on a display of
/// `scale` pixels per point: the width snaps up to [`SIZE_STEP`] and the height
/// keeps the asked aspect.
#[must_use]
pub fn fetch_size(width: u32, height: u32, scale: f32) -> [u32; 2] {
    let want = (width as f32 * scale).clamp(SIZE_STEP, MAX_FETCH_WIDTH);
    let px = ((want / SIZE_STEP).ceil() * SIZE_STEP).min(MAX_FETCH_WIDTH);
    let aspect = height as f32 / width.max(1) as f32;
    [px as u32, (px * aspect).round().max(1.0) as u32]
}

/// The `(cache key, request path)` for one item `thumb` value at `width` x
/// `height`, or `None` when the server value is not an artwork path either
/// source can answer.
///
/// A Jellyfin address names its server and image tag itself ([`crate::source`]),
/// so the request adds only the width; a Plex value goes through
/// [`photo_transcode_path`], which is what keeps a token out of the path.
#[must_use]
pub fn request_for(thumb: &str, width: u32, height: u32) -> Option<(String, String)> {
    let path = if crate::source::split_artwork(thumb).is_some() {
        // A Jellyfin address already names its server and its image tag, so the
        // width the loader wants is all that is added to it — and never a token.
        let join = if thumb.contains('?') { "&" } else { "?" };
        format!("{thumb}{join}maxWidth={}", width.max(1))
    } else {
        photo_transcode_path(thumb, width, height).ok()?
    };
    Some((cache_key(&path, width, height), path))
}

struct Job {
    key: String,
    /// What the source is asked for: a path [`plex_core`] built for Plex, or a
    /// [`crate::source`]-marked Jellyfin route, either way never a token.
    path: String,
    /// The [`Artwork::attach`] generation this job belongs to.
    gen: u64,
    /// The pixel size asked for: the answer is resampled to cover this
    /// before upload, up or down.
    size: [u32; 2],
}

struct Frame {
    width: usize,
    height: usize,
    rgba: Vec<u8>,
    /// The size the answer decoded at, before any resampling: the source's
    /// true size when the request refused a server-side upscale.
    answer: [u32; 2],
}

/// One finished worker attempt: `frame` on success, `error` when it failed.
/// `gen` is the job's generation; an answer for an older server is discarded.
struct Outcome {
    key: String,
    gen: u64,
    frame: Option<Frame>,
    error: Option<String>,
}

struct Shared {
    queue: Mutex<VecDeque<Job>>,
    queued: Condvar,
    /// `None` until a connected session is attached; workers drop jobs until
    /// then so a browse page is not fetched against a server we cannot reach.
    client: Mutex<Option<Arc<LiveClient>>>,
    /// The Jellyfin servers artwork may come from, as the browse side has them.
    /// A separate pool: a Jellyfin poster needs no Plex session, and a Plex
    /// server coming and going leaves these servers where they were.
    jellyfin: Mutex<browse::Jellyfin>,
    /// Bumped on every [`Artwork::attach`]; jobs and answers carry it so work
    /// for a server the UI left is neither fetched nor painted.
    generation: AtomicU64,
    cache: DiskCache,
    /// Cache keys whose answer size has been logged (see [`note_answer`]).
    logged: Mutex<HashSet<String>>,
    closed: AtomicBool,
}

impl Shared {
    fn new(cache_dir: Option<PathBuf>) -> Self {
        Self {
            queue: Mutex::new(VecDeque::new()),
            queued: Condvar::new(),
            client: Mutex::new(None),
            jellyfin: Mutex::new(browse::Jellyfin::default()),
            generation: AtomicU64::new(0),
            cache: DiskCache::new(cache_dir),
            logged: Mutex::new(HashSet::new()),
            closed: AtomicBool::new(false),
        }
    }
}

/// Queue one job, dropping the oldest when the queue is full and freeing that
/// key so a row still on screen can ask again instead of staying a placeholder.
/// Duplicate keys are ignored so a visible row requests once.
fn push(shared: &Shared, inflight: &mut HashSet<String>, job: Job) {
    if inflight.contains(&job.key) {
        return;
    }
    let mut queue = lock(&shared.queue);
    if queue.len() >= QUEUE_CAP {
        if let Some(dropped) = queue.pop_front() {
            inflight.remove(&dropped.key);
        }
    }
    inflight.insert(job.key.clone());
    queue.push_back(job);
    drop(queue);
    shared.queued.notify_one();
}

/// Key-ordered texture set: the least-recently-used entry is freed first.
struct Lru {
    cap: usize,
    budget: usize,
    bytes: usize,
    clock: u64,
    entries: HashMap<String, Entry>,
}

struct Entry {
    handle: TextureHandle,
    used: u64,
    bytes: usize,
}

/// The GPU bytes one RGBA texture takes with its mip chain (~4/3 of the base).
fn texture_bytes(handle: &TextureHandle) -> usize {
    let [w, h] = handle.size();
    w * h * 4 * 4 / 3
}

impl Lru {
    fn new(cap: usize) -> Self {
        Self::with_budget(cap, TEXTURE_BUDGET)
    }

    fn with_budget(cap: usize, budget: usize) -> Self {
        Self {
            cap: cap.max(1),
            budget,
            bytes: 0,
            clock: 0,
            entries: HashMap::new(),
        }
    }

    fn contains(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }

    /// Free every texture: dropping a [`TextureHandle`] is what releases it.
    fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
    }

    /// Marks `key` most-recently-used and returns its texture.
    fn get(&mut self, key: &str) -> Option<&TextureHandle> {
        let used = self.tick();
        self.entries.get_mut(key).map(|entry| {
            entry.used = used;
            &entry.handle
        })
    }

    /// Inserts `handle`, freeing the least-recently-used textures until both
    /// the count cap and the byte budget hold; returns how many were freed.
    fn insert(&mut self, key: String, handle: TextureHandle) -> usize {
        let used = self.tick();
        let bytes = texture_bytes(&handle);
        if let Some(old) = self.entries.remove(&key) {
            self.bytes -= old.bytes;
        }
        let mut freed = 0;
        while !self.entries.is_empty()
            && (self.entries.len() >= self.cap || self.bytes + bytes > self.budget)
        {
            let Some(victim) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.used)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            if let Some(entry) = self.entries.remove(&victim) {
                self.bytes -= entry.bytes;
                freed += 1;
            }
        }
        self.bytes += bytes;
        self.entries.insert(
            key,
            Entry {
                handle,
                used,
                bytes,
            },
        );
        freed
    }

    fn tick(&mut self) -> u64 {
        self.clock += 1;
        self.clock
    }
}

/// The fetch queue, decoder and texture cache behind one browse screen.
pub struct Artwork {
    shared: Arc<Shared>,
    done: Receiver<Outcome>,
    textures: Lru,
    /// Keys already queued or decoding, so a visible row requests once.
    inflight: HashSet<String>,
    /// Mirrors [`Shared::generation`]; only this generation's answers are
    /// uploaded.
    generation: u64,
    /// Keys whose last attempt failed, with the attempts used and when the next
    /// one may be queued, so a rejected poster is not re-requested per frame.
    failed: HashMap<String, Failure>,
    /// First cooldown for a failed key; tests shorten it to exercise the retry.
    retry_base: Duration,
    workers: Vec<JoinHandle<()>>,
    /// Physical pixels per point, read each frame in [`Artwork::drain`].
    scale: f32,
}

/// One key's failure history: attempts so far and the next allowed queue time.
struct Failure {
    attempts: u32,
    retry_at: Instant,
}

impl Artwork {
    /// Loader with an explicit cache directory (`None` disables the disk cache).
    #[must_use]
    pub fn with_cache(cache_dir: Option<PathBuf>, cap: usize) -> Self {
        let (done_tx, done) = channel();
        let shared = Arc::new(Shared::new(cache_dir));
        let workers = (0..WORKERS)
            .filter_map(|index| {
                let shared = Arc::clone(&shared);
                let done_tx = done_tx.clone();
                std::thread::Builder::new()
                    .name(format!("plex-art-{index}"))
                    .spawn(move || worker(&shared, &done_tx))
                    .ok()
            })
            .collect();
        Self {
            shared,
            done,
            textures: Lru::new(cap),
            inflight: HashSet::new(),
            generation: 0,
            failed: HashMap::new(),
            scale: 1.0,
            retry_base: RETRY_BASE,
            workers,
        }
    }

    /// Give the workers a connected session to fetch artwork with, dropping
    /// everything the previous server produced.
    ///
    /// `LiveClient` requests take `&self`, so the loader holds the `Arc` and the
    /// browse worker keeps its own client: artwork never queues behind library or
    /// playback work on that thread. Thumb paths are per-server, so textures and
    /// queued jobs from an earlier server are freed instead of possibly painting
    /// the wrong artwork.
    pub fn attach(&mut self, client: Arc<LiveClient>) {
        self.generation += 1;
        self.shared
            .generation
            .store(self.generation, Ordering::SeqCst);
        *lock(&self.shared.client) = Some(client);
        self.clear();
    }

    /// Give the workers the Jellyfin servers to fetch artwork from. Unlike
    /// [`Artwork::attach`] this frees nothing: a server being added, removed or
    /// re-paired leaves the pictures already on screen alone.
    pub(crate) fn attach_jellyfin(&mut self, jellyfin: browse::Jellyfin) {
        *lock(&self.shared.jellyfin) = jellyfin;
    }

    /// Free the textures, queued jobs and in-flight keys of the server before.
    fn clear(&mut self) {
        self.textures.clear();
        self.inflight.clear();
        self.failed.clear();
        lock(&self.shared.queue).clear();
    }

    /// Upload every finished frame as an egui texture. Call once per frame.
    ///
    /// A failed attempt is recorded instead of dropped: [`Artwork::enqueue`]
    /// then waits out the key's cooldown, so artwork a server rejects is not
    /// re-requested on every frame a row is visible.
    pub fn drain(&mut self, ctx: &Context) {
        self.scale = ctx.pixels_per_point().max(1.0);
        while let Ok(outcome) = self.done.try_recv() {
            if outcome.gen != self.generation {
                continue; // finished after its server was left: never painted
            }
            self.inflight.remove(&outcome.key);
            match outcome.frame {
                Some(frame) => {
                    let image = ColorImage::from_rgba_premultiplied(
                        [frame.width, frame.height],
                        &frame.rgba,
                    );
                    let handle = ctx.load_texture(outcome.key.clone(), image, ART_TEXTURE);
                    self.failed.remove(&outcome.key);
                    self.textures.insert(outcome.key, handle);
                }
                None => {
                    if let Some(error) = outcome.error {
                        log::debug!("artwork fetch failed: {error}");
                    }
                    self.record_failure(&outcome.key);
                }
            }
        }
    }

    /// Count one failed attempt for `key` and set its next allowed queue time.
    fn record_failure(&mut self, key: &str) {
        let base = self.retry_base;
        let failure = self.failed.entry(key.to_owned()).or_insert(Failure {
            attempts: 0,
            retry_at: Instant::now(),
        });
        failure.attempts = failure.attempts.saturating_add(1);
        let backoff = base.saturating_mul(1u32 << failure.attempts.min(10));
        failure.retry_at = Instant::now() + backoff.min(RETRY_CAP);
        if self.failed.len() > MAX_TEXTURES {
            let now = Instant::now();
            self.failed.retain(|_, failure| failure.retry_at > now);
        }
    }

    /// The `(cache key, request path)` poster draws for one item `thumb` at
    /// [`POSTER`] size, or `None` when the server value is not an artwork path
    /// [`plex_core`] will request.
    #[must_use]
    fn poster_request(thumb: &str) -> Option<(String, String)> {
        request_for(thumb, u32::from(POSTER[0]), u32::from(POSTER[1]))
    }

    /// Draw the poster for one item `thumb`, or a card placeholder while it is
    /// queued, decoding, unavailable or rejected.
    pub fn poster(&mut self, ui: &mut Ui, thumb: Option<&str>) -> Response {
        let size = Vec2::new(f32::from(POSTER[0]), f32::from(POSTER[1]));
        // A row scrolled out of view asks for nothing.
        if !ui.is_rect_visible(egui::Rect::from_min_size(ui.cursor().min, size)) {
            return placeholder(ui, size);
        }
        let request = thumb.and_then(Self::poster_request);
        if let Some((key, path)) = &request {
            self.enqueue(
                key.clone(),
                path.clone(),
                [u32::from(POSTER[0]), u32::from(POSTER[1])],
            );
        }
        let texture = request
            .as_ref()
            .and_then(|(key, _)| self.textures.get(key).map(|h| (h.id(), h.size_vec2())));
        match texture {
            Some((id, _)) => ui.image((id, size)),
            None => placeholder(ui, size),
        }
    }

    /// Request `thumb` drawn at `width` x `height` points (fetched at the display's
    /// pixel density, see [`fetch_size`]) and draw with the caller's own
    /// rect/uv: returns `(texture, texture size)` once the loader has it, and
    /// `None` while it is queued, decoding or rejected. Cache keys carry the
    /// requested size, so a card, a poster and the hero never share a frame.
    pub fn texture(
        &mut self,
        thumb: Option<&str>,
        width: u32,
        height: u32,
    ) -> Option<(egui::TextureId, Vec2)> {
        let [width, height] = fetch_size(width, height, self.scale);
        let (key, path) = thumb.and_then(|thumb| request_for(thumb, width, height))?;
        self.enqueue(key.clone(), path.clone(), [width, height]);
        self.textures
            .get(&key)
            .map(|handle| (handle.id(), handle.size_vec2()))
    }

    fn enqueue(&mut self, key: String, path: String, size: [u32; 2]) {
        if self.textures.contains(&key) {
            return;
        }
        if self.failed.get(&key).is_some_and(|failure| {
            failure.attempts >= MAX_ATTEMPTS || Instant::now() < failure.retry_at
        }) {
            return;
        }
        push(
            &self.shared,
            &mut self.inflight,
            Job {
                key,
                path,
                gen: self.generation,
                size,
            },
        );
    }

    /// True when `key` has a texture (test view).
    #[cfg(test)]
    fn has_texture(&self, key: &str) -> bool {
        self.textures.contains(key)
    }

    #[cfg(test)]
    fn texture_count(&self) -> usize {
        self.textures.entries.len()
    }

    #[cfg(test)]
    fn queued_len(&self) -> usize {
        lock(&self.shared.queue).len()
    }

    #[cfg(test)]
    fn has_inflight(&self, key: &str) -> bool {
        self.inflight.contains(key)
    }

    /// Tests shorten the retry cooldown to exercise the backoff without waiting.
    #[cfg(test)]
    fn set_retry_base(&mut self, base: Duration) {
        self.retry_base = base;
    }

    /// Attempts recorded for `key` (test view).
    #[cfg(test)]
    fn failure_attempts(&self, key: &str) -> u32 {
        self.failed.get(key).map_or(0, |failure| failure.attempts)
    }

    /// Keys with a recorded failure (test view).
    #[cfg(test)]
    fn failure_count(&self) -> usize {
        self.failed.len()
    }

    /// Tests wait for a worker's failed attempt (no client attached) to reach
    /// [`Artwork::drain`]: the helper waits until `at_least` attempts are
    /// recorded, so an attempt already counted is not mistaken for the new one.
    #[cfg(test)]
    fn wait_for_failure(&mut self, key: &str, at_least: u32) -> bool {
        let ctx = Context::default();
        for _ in 0..100 {
            self.drain(&ctx);
            if self.failure_attempts(key) >= at_least {
                return true;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        false
    }

    #[cfg(test)]
    fn inflight_count(&self) -> usize {
        self.inflight.len()
    }

    /// Tests wait for a worker to deliver without a server by pre-filling the
    /// disk cache.
    #[cfg(test)]
    fn wait_for_texture(&mut self, ctx: &Context, key: &str) -> bool {
        for _ in 0..100 {
            self.drain(ctx);
            if self.has_texture(key) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        false
    }
}

impl Default for Artwork {
    /// The app loader: disk cache under `$XDG_CACHE_HOME`, [`MAX_TEXTURES`] cap.
    fn default() -> Self {
        Self::with_cache(env_dir(), MAX_TEXTURES)
    }
}

impl Drop for Artwork {
    fn drop(&mut self) {
        self.shared.closed.store(true, Ordering::SeqCst);
        self.shared.queued.notify_all();
        for handle in self.workers.drain(..) {
            let _ = handle.join();
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// True when `job` belongs to the server the loader is attached to now: thumb
/// paths are per-server, so work for a server the UI left is dropped instead of
/// fetched against the session that replaced it.
fn current(shared: &Shared, job: &Job) -> bool {
    job.gen == shared.generation.load(Ordering::SeqCst)
}

fn worker(shared: &Shared, done: &Sender<Outcome>) {
    while let Some(job) = next_job(shared) {
        if !current(shared, &job) {
            continue;
        }
        let client = lock(&shared.client).clone();
        let outcome = fetch(shared, client.as_ref(), &job);
        let _ = done.send(outcome);
        crate::watch::wake();
    }
}

fn next_job(shared: &Shared) -> Option<Job> {
    let mut queue = lock(&shared.queue);
    loop {
        if let Some(job) = queue.pop_front() {
            return Some(job);
        }
        if shared.closed.load(Ordering::SeqCst) {
            return None;
        }
        let (guard, _waited) = shared
            .queued
            .wait_timeout(queue, QUEUE_WAIT)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        queue = guard;
    }
}

/// Decode an answer for `job` and log once per key what size it came in at.
fn decode_logged(shared: &Shared, job: &Job, bytes: &[u8]) -> Option<Frame> {
    let frame = decode(bytes, job.size)?;
    note_answer(shared, &job.key, job.size, frame.answer);
    Some(frame)
}

/// Log the requested size against the decoded answer size, once per cache
/// key, at debug level: `RUST_LOG=plex_app=debug` then shows which items have
/// art smaller than the screen asks for. The key is a content-free digest, so
/// the line carries no URL, query or token.
fn note_answer(shared: &Shared, key: &str, asked: [u32; 2], answer: [u32; 2]) {
    let mut logged = lock(&shared.logged);
    if logged.len() >= LOGGED_KEYS_CAP {
        logged.clear();
    }
    if logged.insert(key.to_owned()) {
        log::debug!(
            "artwork {key}: asked {}x{}, answer decoded {}x{}",
            asked[0],
            asked[1],
            answer[0],
            answer[1]
        );
    }
}

fn fetch(shared: &Shared, client: Option<&Arc<LiveClient>>, job: &Job) -> Outcome {
    if let Some(frame) = shared
        .cache
        .read(&job.key)
        .and_then(|bytes| decode_logged(shared, job, &bytes))
    {
        return Outcome {
            key: job.key.clone(),
            gen: job.gen,
            frame: Some(frame),
            error: None,
        };
    }
    // A Jellyfin route names the server it belongs to, so its poster needs no
    // Plex session; a Plex path is asked of the connected session.
    let answer = match browse::path_server(&job.path) {
        Some(server) => lock(&shared.jellyfin).image(server, &job.path),
        None => match client {
            Some(client) => client
                .raw_get(&job.path, &[])
                .map_err(|error| error.to_string()),
            None => {
                return Outcome {
                    key: job.key.clone(),
                    gen: job.gen,
                    frame: None,
                    error: Some("no connected server yet".to_owned()),
                }
            }
        },
    };
    match answer {
        Ok((status, body)) if status == 200 && !body.is_empty() => {
            match decode_logged(shared, job, &body) {
                Some(frame) => {
                    shared.cache.write(&job.key, &body);
                    Outcome {
                        key: job.key.clone(),
                        gen: job.gen,
                        frame: Some(frame),
                        error: None,
                    }
                }
                None => Outcome {
                    key: job.key.clone(),
                    gen: job.gen,
                    frame: None,
                    error: Some(format!("could not decode {} bytes of artwork", body.len())),
                },
            }
        }
        Ok((status, _)) => Outcome {
            key: job.key.clone(),
            gen: job.gen,
            frame: None,
            error: Some(format!("artwork request returned {status}")),
        },
        Err(error) => Outcome {
            key: job.key.clone(),
            gen: job.gen,
            frame: None,
            error: Some(error),
        },
    }
}

/// Decode a `/photo/:/transcode` answer into premultiplied RGBA pixels
/// resampled to cover `target`, up or down (see [`resample`]).
///
/// The answer size and the decoder's own dimension/allocation limits are capped,
/// so server-supplied bytes decide neither how much is read nor how large the
/// `width * height * 4` pixel buffer may get.
fn decode(bytes: &[u8], target: [u32; 2]) -> Option<Frame> {
    if bytes.len() > MAX_ENCODED_BYTES {
        log::debug!(
            "artwork answer of {} bytes is over the decode limit",
            bytes.len()
        );
        return None;
    }
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DECODE_DIM);
    limits.max_image_height = Some(MAX_DECODE_DIM);
    limits.max_alloc = Some(MAX_DECODE_ALLOC);
    reader.limits(limits);
    let image = reader.decode().ok()?;
    let mut rgba = image.to_rgba8();
    let (answer_w, answer_h) = rgba.dimensions();
    premultiply(&mut rgba);
    let rgba = resample(rgba, target);
    let frame = Frame {
        width: usize::try_from(rgba.width()).unwrap_or(0),
        height: usize::try_from(rgba.height()).unwrap_or(0),
        rgba: rgba.into_raw(),
        answer: [answer_w, answer_h],
    };
    if frame.width == 0 || frame.height == 0 {
        return None;
    }
    Some(frame)
}

/// Multiply each pixel's colour by its alpha, as egui's textures expect. Done
/// before any resampling too, so transparent pixels lend no colour to the edges
/// of a logo.
fn premultiply(image: &mut image::RgbaImage) {
    for pixel in image.pixels_mut() {
        let [r, g, b, a] = pixel.0;
        let mul = |c: u8| ((u16::from(c) * u16::from(a) + 127) / 255) as u8;
        pixel.0 = [mul(r), mul(g), mul(b), a];
    }
}

/// `image` scaled with Lanczos3 to the smallest size that still covers
/// `target`, in either direction, when the difference is meaningful.
///
/// Downsizing matters because a server that sends more than was asked would
/// otherwise leave the GPU's mipmaps to shrink it, which softens fine detail
/// such as a title logo's lettering. Upsizing matters because the request
/// refuses server-side upscaling, so small source art arrives small and the
/// GPU's bilinear magnification would stretch a 1920px backdrop to 3840px on
/// its own. The resized buffer is bounded by [`MAX_DECODE_ALLOC`]; past that
/// the answer is left as it is for the GPU to scale.
fn resample(image: image::RgbaImage, target: [u32; 2]) -> image::RgbaImage {
    let (w, h) = image.dimensions();
    let scale = (target[0] as f32 / w.max(1) as f32).max(target[1] as f32 / h.max(1) as f32);
    if (DOWNSCALE_FROM..=UPSCALE_BELOW).contains(&scale) {
        return image;
    }
    let width = ((w as f32 * scale).round() as u32).max(1);
    let height = ((h as f32 * scale).round() as u32).max(1);
    if width == w && height == h {
        return image;
    }
    if u64::from(width) * u64::from(height) * 4 > MAX_DECODE_ALLOC {
        return image;
    }
    image::imageops::resize(&image, width, height, image::imageops::FilterType::Lanczos3)
}

/// The card drawn while artwork is missing: `CARD` filled at the card radius.
fn placeholder(ui: &mut Ui, size: Vec2) -> Response {
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter()
        .rect_filled(rect, theme::CARD_RADIUS, theme::CARD);
    response
}

#[cfg(test)]
mod tests;
