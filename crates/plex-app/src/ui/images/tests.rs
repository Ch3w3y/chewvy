//! Loader tests: request/cache-key shape, queue and texture bounds, the disk
//! cache path, and that artwork work never names a token.

use std::path::Path;
use std::time::Duration;

use super::*;
use crate::source;
use egui::{ColorImage, Context, TextureOptions};

/// A `thumb` value shaped like the ones the captured server answers name.
const THUMB: &str = "/library/metadata/36100/thumb/1778807077";

/// The request `/photo/:/transcode` path for a real thumb value at poster size.
fn request_path(thumb: &str) -> String {
    match photo_transcode_path(thumb, u32::from(POSTER[0]), u32::from(POSTER[1])) {
        Ok(path) => path,
        Err(error) => panic!("a server-named thumb value must be requestable: {error}"),
    }
}

fn texture(ctx: &Context, name: &str) -> TextureHandle {
    ctx.load_texture(
        name,
        ColorImage::from_rgb([1, 1], &[1, 2, 3]),
        TextureOptions::NEAREST,
    )
}

/// A 1x1 PNG written straight into one loader cache slot.
#[must_use]
fn write_cached_png(dir: &Path, key: &str) -> usize {
    let path = dir.join(crate::ui::art_cache::file_name(key));
    assert!(
        image::save_buffer_with_format(
            &path,
            &[0u8, 128, 255, 255],
            1,
            1,
            image::ColorType::Rgba8,
            image::ImageFormat::Png,
        )
        .is_ok(),
        "a 1x1 png cache slot must be writable"
    );
    assert!(path.is_file(), "the cache slot must exist");
    DiskCache::new(Some(dir.to_path_buf()))
        .read(key)
        .map_or(0, |bytes| bytes.len())
}

#[must_use]
fn temp_cache_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("chewvy-test-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        std::fs::create_dir_all(&dir).is_ok(),
        "the test cache directory must be creatable"
    );
    dir
}

fn poster_pair() -> (String, String) {
    let Some(pair) = Artwork::poster_request(THUMB) else {
        panic!("a server-named thumb value must be requestable at poster size");
    };
    pair
}

#[test]
fn cache_key_is_stable_and_specific_to_the_requested_size() {
    let path = request_path(THUMB);
    let key = cache_key(&path, 60, 90);
    assert_eq!(key, cache_key(&path, 60, 90));
    assert_eq!(key.len(), 64);
    assert!(key.chars().all(|c| c.is_ascii_hexdigit()));
    assert_ne!(key, cache_key(&path, 120, 180));
    assert_ne!(key, cache_key(&path, 60, 91));
    assert_ne!(
        key,
        cache_key(&request_path("/library/metadata/9/thumb/1"), 60, 90)
    );
}

#[test]
fn the_request_path_is_relative_and_form_encodes_the_thumb_into_one_parameter() {
    let path = request_path(THUMB);
    assert!(path.starts_with("/photo/:/transcode?"));
    assert!(path.contains(&format!("width={}&height={}", POSTER[0], POSTER[1])));
    assert!(path.contains("url=%2Flibrary%2Fmetadata%2F36100%2Fthumb%2F1778807077"));
    assert!(!path.contains("://"));
    assert!(!path.contains("X-Plex-Token"));
}

#[test]
fn a_thumb_naming_the_token_is_never_queued_or_cached() {
    let rejected = [
        "/library/metadata/1/thumb/2?X-Plex-Token=supersecret",
        "/library/metadata/1/thumb/2#X-Plex-Token=supersecret",
        "/library/metadata/1/thumb/2%3AX-Plex-Token",
        "X-Plex-Token=supersecret",
        "/library/metadata/1/thumb/2 X-Plex-Token=supersecret",
        "/library/metadata/1\tthumb",
        "/library/metadata/\u{7}",
        "library/metadata/1/thumb/2",
        "",
        &format!("/library/metadata/{}", "9".repeat(2048)),
    ];
    let mut artwork = Artwork::with_cache(None, MAX_TEXTURES);
    for thumb in rejected {
        if let Some((key, path)) = Artwork::poster_request(thumb) {
            artwork.enqueue(key, path, TEST_SIZE);
        }
    }
    assert_eq!(artwork.queued_len(), 0);
    assert_eq!(artwork.inflight_count(), 0);
    assert_eq!(artwork.texture_count(), 0);
    assert_eq!(artwork.failure_count(), 0, "a rejected thumb never asks");
}

#[test]
fn the_queue_holds_only_validated_paths_and_keys_derived_from_them() {
    let shared = Shared::new(None);
    let mut inflight = HashSet::new();
    for thumb in [THUMB, "/library/metadata/9/thumb/1", THUMB] {
        let Some((key, path)) = Artwork::poster_request(thumb) else {
            panic!("{thumb:?} must be requestable at poster size");
        };
        push(
            &shared,
            &mut inflight,
            Job {
                key,
                path,
                gen: 0,
                size: TEST_SIZE,
            },
        );
    }
    let queue = lock(&shared.queue);
    assert_eq!(queue.len(), 2, "a visible row requests once");
    for job in queue.iter() {
        assert!(job.path.starts_with("/photo/:/transcode?"));
        assert!(!job.path.contains("X-Plex-Token"));
        assert!(!job.path.contains("supersecret"));
        assert_eq!(job.key, cache_key(&job.path, 60, 90));
    }
    assert_eq!(inflight.len(), 2);
    let (key, _) = poster_pair();
    assert!(inflight.contains(&key));
}

#[test]
fn a_full_queue_drops_the_oldest_job_and_frees_its_key() {
    let shared = Shared::new(None);
    let mut inflight = HashSet::new();
    let thumbs: Vec<String> = (0..=QUEUE_CAP)
        .map(|i| format!("/library/metadata/{i}/thumb/1"))
        .collect();
    for thumb in &thumbs {
        let Some((key, path)) = request_for(thumb, 60, 90) else {
            panic!("{thumb:?} must be requestable");
        };
        push(
            &shared,
            &mut inflight,
            Job {
                key,
                path,
                gen: 0,
                size: TEST_SIZE,
            },
        );
    }
    let oldest = thumbs
        .first()
        .and_then(|thumb| request_for(thumb, 60, 90))
        .map(|(key, _)| key);
    let newest = thumbs.last().and_then(|thumb| request_for(thumb, 60, 90));
    let (Some(oldest), Some((newest_key, _))) = (oldest, newest) else {
        panic!("the oldest and newest thumbs must be requestable");
    };
    let queue = lock(&shared.queue);
    assert_eq!(queue.len(), QUEUE_CAP);
    assert!(!queue.iter().any(|job| job.key == oldest));
    assert!(!inflight.contains(&oldest), "a dropped key is free");
    assert!(queue.iter().any(|job| job.key == newest_key));
    assert!(inflight.contains(&newest_key));
}

#[test]
fn textures_stop_at_the_cap_and_the_least_recently_used_frees_first() {
    let ctx = Context::default();
    let mut lru = Lru::new(MAX_TEXTURES);
    for i in 0..MAX_TEXTURES {
        let key = format!("k{i}");
        lru.insert(key.clone(), texture(&ctx, &key));
    }
    assert_eq!(lru.entries.len(), MAX_TEXTURES);
    // Touch the first entry, so the second becomes the eviction victim.
    assert!(lru.get("k0").is_some());
    let freed = lru.insert("new".to_owned(), texture(&ctx, "new"));
    assert_eq!(lru.entries.len(), MAX_TEXTURES, "the cap holds");
    assert_eq!(freed, 1, "the LRU texture is freed first");
    assert!(!lru.contains("k1"));
    assert!(lru.contains("k0"));
    assert!(lru.contains("new"));
}

#[test]
fn the_byte_budget_frees_the_oldest_textures_first() {
    let ctx = Context::default();
    let one = super::texture_bytes(&texture(&ctx, "probe"));
    let mut lru = Lru::with_budget(MAX_TEXTURES, one * 3);
    for key in ["a", "b", "c"] {
        lru.insert(key.to_owned(), texture(&ctx, key));
    }
    assert!(lru.get("a").is_some());
    assert_eq!(lru.insert("d".to_owned(), texture(&ctx, "d")), 1);
    assert!(!lru.contains("b"), "the least recently used goes");
    assert!(lru.contains("a") && lru.contains("c") && lru.contains("d"));
    assert_eq!(lru.bytes, one * 3);
}

#[test]
fn fetch_sizes_follow_the_display_density_in_steps() {
    use super::fetch_size;
    assert_eq!(fetch_size(160, 240, 1.0), [192, 288]);
    assert_eq!(fetch_size(160, 240, 1.5), [256, 384]);
    assert_eq!(fetch_size(1600, 900, 2.0), [3200, 1800]);
    assert_eq!(fetch_size(4000, 2250, 2.0)[0], 3840, "capped");
}

#[test]
fn a_job_for_a_left_server_is_not_fetched() {
    let shared = Shared::new(None);
    let job = Job {
        key: "k".to_owned(),
        path: request_path(THUMB),
        gen: 0,
        size: TEST_SIZE,
    };
    assert!(current(&shared, &job));
    shared.generation.store(1, Ordering::SeqCst);
    assert!(!current(&shared, &job));
}

#[test]
fn a_cached_answer_uploads_without_a_connected_server() {
    let ctx = Context::default();
    let (key, path) = poster_pair();
    let dir = temp_cache_dir("art-cache");
    assert!(write_cached_png(&dir, &key) > 0);
    let mut artwork = Artwork::with_cache(Some(dir), MAX_TEXTURES);
    artwork.enqueue(key.clone(), path, TEST_SIZE);
    assert!(
        artwork.wait_for_texture(&ctx, &key),
        "a cached poster must reach the texture cache without any request"
    );
    assert_eq!(artwork.texture_count(), 1);
}

#[test]
fn clearing_frees_textures_queue_and_keys() {
    let ctx = Context::default();
    let (key, path) = poster_pair();
    let dir = temp_cache_dir("art-clear");
    assert!(write_cached_png(&dir, &key) > 0);
    let mut artwork = Artwork::with_cache(Some(dir), MAX_TEXTURES);
    artwork.enqueue(key.clone(), path, TEST_SIZE);
    assert!(artwork.has_inflight(&key));
    assert!(artwork.wait_for_texture(&ctx, &key));
    assert_eq!(artwork.texture_count(), 1);
    artwork.clear();
    assert_eq!(artwork.texture_count(), 0);
    assert_eq!(artwork.queued_len(), 0);
    assert_eq!(artwork.inflight_count(), 0);
}

#[test]
fn a_failed_poster_waits_out_its_cooldown_instead_of_being_requeued() {
    let (key, path) = poster_pair();
    let mut artwork = Artwork::with_cache(None, MAX_TEXTURES);
    artwork.set_retry_base(Duration::from_millis(150));
    artwork.enqueue(key.clone(), path.clone(), TEST_SIZE);
    // A worker may take the job before it is read back, so the request is
    // observed through the loader's own in-flight key, not the queue length.
    assert!(artwork.has_inflight(&key), "the first request is queued");
    // A worker takes the job; without a client the attempt answers with a failure.
    assert!(artwork.wait_for_failure(&key, 1));
    assert_eq!(artwork.failure_attempts(&key), 1);
    artwork.enqueue(key.clone(), path, TEST_SIZE);
    assert_eq!(artwork.queued_len(), 0, "the cooldown blocks a re-request");
    assert!(!artwork.has_inflight(&key));
}

#[test]
fn a_key_the_loader_gives_up_on_is_retired_until_the_next_attach() {
    let (key, path) = poster_pair();
    let mut artwork = Artwork::with_cache(None, MAX_TEXTURES);
    artwork.set_retry_base(Duration::ZERO);
    for attempt in 1..=MAX_ATTEMPTS {
        artwork.enqueue(key.clone(), path.clone(), TEST_SIZE);
        assert!(artwork.wait_for_failure(&key, attempt));
        assert_eq!(artwork.failure_attempts(&key), attempt);
    }
    std::thread::sleep(Duration::from_millis(10));
    artwork.enqueue(key.clone(), path.clone(), TEST_SIZE);
    assert_eq!(
        artwork.queued_len(),
        0,
        "MAX_ATTEMPTS failures retire the key"
    );
    // [`Artwork::attach`] clears the failure history of the server before.
    artwork.clear();
    assert_eq!(artwork.failure_attempts(&key), 0);
    artwork.enqueue(key.clone(), path, TEST_SIZE);
    assert!(artwork.has_inflight(&key), "a fresh session may retry");
}

#[test]
fn oversized_and_undecodable_answers_are_refused_before_pixels() {
    assert!(decode(&vec![0u8; MAX_ENCODED_BYTES + 1], TEST_SIZE).is_none());
    assert!(decode(b"not an image at all", TEST_SIZE).is_none());
    assert!(decode(&[], TEST_SIZE).is_none());
    // A 1x1 PNG decodes, the format of a real `/photo/:/transcode` answer.
    let dir = temp_cache_dir("art-decode");
    let file = dir.join("probe.png");
    assert!(image::save_buffer_with_format(
        &file,
        &[0u8, 128, 255, 255],
        1,
        1,
        image::ColorType::Rgba8,
        image::ImageFormat::Png,
    )
    .is_ok());
    let png = std::fs::read(&file).ok();
    assert!(png.is_some(), "the written png must be readable");
    assert!(
        decode(&png.unwrap_or_default(), [1, 1])
            .is_some_and(|frame| frame.width == 1 && frame.height == 1),
        "a real png answer must decode to its own size"
    );
}

/// A request size larger than every test image, so nothing is rescaled.
const TEST_SIZE: [u32; 2] = [64, 64];

fn png_bytes(width: u32, height: u32, pixel: [u8; 4]) -> Vec<u8> {
    let pixels: Vec<u8> = pixel.repeat((width * height) as usize);
    let mut out = std::io::Cursor::new(Vec::new());
    let written = image::write_buffer_with_format(
        &mut out,
        &pixels,
        width,
        height,
        image::ColorType::Rgba8,
        image::ImageFormat::Png,
    );
    assert!(written.is_ok());
    out.into_inner()
}

#[test]
fn decoded_pixels_are_premultiplied_as_egui_expects() {
    let frame = decode(&png_bytes(1, 1, [255, 255, 255, 128]), [1, 1]);
    assert_eq!(frame.map(|f| f.rgba), Some(vec![128, 128, 128, 128]));
}

#[test]
fn an_oversized_answer_is_scaled_down_to_cover_the_request() {
    let frame = decode(&png_bytes(400, 100, [10, 20, 30, 255]), [100, 20]);
    assert_eq!(frame.map(|f| (f.width, f.height)), Some((100, 25)));
    // Within the threshold it is left for the GPU.
    let frame = decode(&png_bytes(110, 20, [10, 20, 30, 255]), [100, 20]);
    assert_eq!(frame.map(|f| (f.width, f.height)), Some((110, 20)));
}

#[test]
fn a_smaller_answer_is_scaled_up_to_cover_the_request() {
    // upscale=0 means a small source arrives at its own size; a worker
    // resamples it rather than leaving a 2x stretch to the GPU's magnification.
    let frame = decode(&png_bytes(128, 72, [10, 20, 30, 255]), [256, 144]);
    assert_eq!(frame.map(|f| (f.width, f.height)), Some((256, 144)));
}

#[test]
fn an_answer_at_the_requested_size_is_left_alone() {
    let frame = decode(&png_bytes(100, 50, [10, 20, 30, 255]), [100, 50]);
    assert_eq!(frame.map(|f| (f.width, f.height)), Some((100, 50)));
    // The answer size is reported before any resampling, which is what the
    // once-per-key debug log compares against the request.
    let frame = decode(&png_bytes(128, 72, [10, 20, 30, 255]), [256, 144]);
    assert_eq!(frame.map(|f| f.answer), Some([128, 72]));
}

#[test]
fn an_upscale_over_the_decode_budget_is_left_to_the_gpu() {
    // The cover scale is 4096/1200, so 1600x1200 would resize to a buffer
    // over MAX_DECODE_ALLOC pixels; the answer is passed through untouched.
    let frame = decode(&png_bytes(1600, 1200, [10, 20, 30, 255]), [4096, 4096]);
    assert_eq!(frame.map(|f| (f.width, f.height)), Some((1600, 1200)));
}

/// What [`request_for`] makes of an address, or this test's own failure.
fn asked(address: &str, width: u32, height: u32) -> (String, String) {
    match request_for(address, width, height) {
        Some(request) => request,
        None => panic!("`{address}` is requestable"),
    }
}

/// A Jellyfin server's own artwork address, as [`crate::source`] marks it: its
/// server, its route, and the image tag the server names for it.
const JELLYFIN_ART: &str = "jellyfin:0f1e/Items/323fe870/Images/Primary?tag=f3bf";

/// The brief's artwork rule: Jellyfin images go through this loader, sized, with
/// the token only in the `Authorization` header — which is what the request path
/// must therefore be free of.
#[test]
fn a_jellyfin_address_is_asked_at_its_width_with_its_tag_and_no_token() {
    let (key, path) = asked(JELLYFIN_ART, u32::from(POSTER[0]), u32::from(POSTER[1]));
    assert_eq!(
        path,
        format!("{JELLYFIN_ART}&maxWidth={}", u32::from(POSTER[0]))
    );
    assert_eq!(
        key,
        cache_key(&path, u32::from(POSTER[0]), u32::from(POSTER[1]))
    );
    let lowered = path.to_lowercase();
    for name in [
        "token",
        "auth",
        "apikey",
        "api_key",
        "access_token",
        "secret",
    ] {
        assert!(!lowered.contains(name), "{name} in {path}");
    }
    // The route is what the named server hears, and the loader knows to ask it
    // there rather than the connected Plex session.
    assert_eq!(browse::path_server(&path), Some("0f1e"));
    // The server hears its own route; the width is the only thing added to it.
    assert_eq!(
        source::split_artwork(JELLYFIN_ART),
        Some(("0f1e", "/Items/323fe870/Images/Primary?tag=f3bf"))
    );
}

#[test]
fn a_jellyfin_address_is_also_a_request_when_no_width_was_asked() {
    let (_, path) = asked(JELLYFIN_ART, 0, 0);
    assert!(path.ends_with("&maxWidth=1"));
    // A Plex thumb value is still the other source's shape of request.
    assert_eq!(browse::path_server(&request_path(THUMB)), None);
}
