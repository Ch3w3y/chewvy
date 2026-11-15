//! The catalog read a theme's background art is chosen from, against a 12.4
//! server whose library carries real TMDb metadata (`fixtures/live-12.4-catalog/`,
//! captured with exactly the request [`Client::catalog`] sends): the ratings,
//! their two scales, the original title, the provider ids and the backdrops by
//! index.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use jellyfin_core::browse::{self, Client, ItemKind, CATALOG_FIELDS};
use jellyfin_core::dto::{ImageInfo, Item, QueryResult};
use jellyfin_core::{DeviceInfo, Http, SecretString};

mod mock;

const FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/live-12.4-catalog/"
);
const USER_ID: &str = "70eb2746aca443c098159c5d6e66d886";
const ODYSSEY_ID: &str = "21d49f90ff283515661cc97b3b13e1a9";

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!("{FIXTURES}{name}.json")).expect("fixture present")
}

fn catalog_page() -> QueryResult<Item> {
    serde_json::from_str(&fixture("catalog_page")).expect("the catalog capture parses")
}

#[test]
fn a_catalog_row_names_both_ratings_on_their_own_scales() {
    let page = catalog_page();
    assert_eq!(
        page.total_record_count,
        Some(32),
        "the whole catalog's size"
    );
    assert_eq!(page.items.len(), 3, "the page of three it was asked for");
    let odyssey = &page.items[1];
    assert_eq!(odyssey.name.as_deref(), Some("2001: A Space Odyssey"));
    assert_eq!(odyssey.production_year, Some(1968));
    // TMDb's vote average, out of 10, and the Tomatometer, out of 100.
    assert_eq!(odyssey.community_rating, Some(8.042));
    assert_eq!(odyssey.critic_rating, Some(91.0));
    assert_eq!(
        odyssey.original_title.as_deref(),
        Some("2001: A Space Odyssey")
    );
    assert_eq!(
        odyssey.genres,
        ["Science Fiction", "Mystery", "Adventure"].map(str::to_owned)
    );
    assert_eq!(
        odyssey.provider_ids.get("Imdb").map(String::as_str),
        Some("tt0062622")
    );
    assert_eq!(
        odyssey.provider_ids.get("Tmdb").map(String::as_str),
        Some("62")
    );
    assert!(
        odyssey.tags.iter().any(|tag| tag == "space travel"),
        "TMDb's keywords arrive as Tags: {:?}",
        odyssey.tags
    );
    assert_eq!(odyssey.backdrop_image_tags.len(), 1);
    assert!(
        page.items.iter().all(|row| row.community_rating.is_some()
            && row.critic_rating.is_some()
            && !row.backdrop_image_tags.is_empty()),
        "every row of the capture is rated and has a backdrop"
    );
}

#[test]
fn a_provider_the_server_names_no_id_for_is_left_out() {
    let row: Item = serde_json::from_str(
        r#"{"Id":"21d49f90ff283515661cc97b3b13e1a9","ProviderIds":{"Imdb":"tt0062622","Tvdb":null,"Tmdb":""}}"#,
    )
    .expect("a null provider id is not a shape error");
    assert_eq!(row.provider_ids.len(), 1);
    let bare: Item = serde_json::from_str(r#"{"Id":"x","ProviderIds":null}"#).expect("null map");
    assert!(bare.provider_ids.is_empty());
}

#[test]
fn the_images_answer_sizes_each_backdrop_by_its_index() {
    let images: Vec<ImageInfo> =
        serde_json::from_str(&fixture("catalog_images")).expect("the images capture parses");
    let backdrops: Vec<&ImageInfo> = images
        .iter()
        .filter(|image| image.image_type.as_deref() == Some("Backdrop"))
        .collect();
    assert_eq!(backdrops.len(), 1);
    let backdrop = backdrops[0];
    assert_eq!(backdrop.image_index, Some(0));
    assert_eq!((backdrop.width, backdrop.height), (Some(3840), Some(2160)));
    let row = &catalog_page().items[1];
    assert_eq!(
        backdrop.image_tag.as_deref(),
        row.backdrop_image_tags.first().map(String::as_str),
        "the tag the catalog row names is the one the images answer sizes"
    );
    // A single-image type has no index to name.
    assert!(images
        .iter()
        .filter(|image| image.image_type.as_deref() == Some("Primary"))
        .all(|image| image.image_index.is_none()));
}

#[test]
fn each_backdrop_is_addressed_by_its_position() {
    assert_eq!(
        browse::backdrop_address(ODYSSEY_ID, 2, "beef").as_deref(),
        Some("/Items/21d49f90ff283515661cc97b3b13e1a9/Images/Backdrop/2?tag=beef")
    );
    assert_eq!(
        browse::backdrop_address(ODYSSEY_ID, 0, "beef"),
        browse::image_address(ODYSSEY_ID, browse::ImageKind::Backdrop, "beef"),
        "the first backdrop has one address whichever builder made it"
    );
    assert_eq!(browse::backdrop_address("../Users", 0, "beef"), None);
}

#[test]
fn a_catalog_page_is_a_lean_paged_read_of_every_view() {
    let server = mock::MockJfServer::start();
    server.route(
        "GET /Items",
        mock::Answer::json(200, fixture("catalog_page")),
    );
    let http = Http::with_timeouts(
        &server.url(),
        std::time::Duration::from_secs(5),
        std::time::Duration::from_secs(5),
    )
    .expect("mock base");
    let device = DeviceInfo {
        app: "Chewvy".to_owned(),
        app_version: "0.1.0".to_owned(),
        device: "test-machine".to_owned(),
        device_id: "0a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d".to_owned(),
    };
    let token = SecretString::new("c8f0a1d2e3b4456789abcdef01234567");
    let client = Client::new(&http, &device, &token, USER_ID).expect("a usable user id");

    let page = client
        .catalog(&[ItemKind::Movie, ItemKind::Series], 50, 3)
        .expect("a catalog page");
    assert_eq!(page.items.len(), 3);

    let recorded = server.requests().first().expect("one request").clone();
    let fields = CATALOG_FIELDS.replace(',', "%2C");
    assert_eq!(
        recorded.path_and_query,
        format!(
            "/Items?userId={USER_ID}&recursive=true&includeItemTypes=Movie%2CSeries\
             &sortBy=SortName&startIndex=50&limit=3&fields={fields}&enableImageTypes=Backdrop"
        ),
        "no parentId: the catalog spans every view"
    );
    assert_eq!(recorded.method, "GET");
    assert!(!recorded.path_and_query.contains(token.expose()));
    assert_eq!(
        recorded.authorization.as_deref(),
        Some(jellyfin_core::media_browser_header(&device, Some(&token)).as_str()),
        "the token rides the header and nothing else"
    );
    assert!(recorded.body.is_empty(), "a read carries no body");
}
