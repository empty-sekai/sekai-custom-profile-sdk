//! `sekai-profile`: renders Project SEKAI custom profile cards and prepares
//! the SDF atlases the renderer samples.
//!
//! - `render` draws one page to a PNG or JPEG file, first adding any glyph
//!   or shape the page needs to the atlas directory.
//! - `prepare` builds complete atlases for every font family and shape.
//! - `serve` answers render requests read as NDJSON from stdin.

mod fetch;
mod serve;
mod static_manifest;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use clap::{Args, Parser, Subcommand, ValueEnum};
use sekai_profile::assets::AssetStore;
use sekai_profile::atlas_dir::AtlasDir;
use sekai_profile::masterdata::JsonMasterDataProvider;
use sekai_profile::prepare::{prepare_atlas_dir, PrepareOptions, PrepareReport, Prepared};
use sekai_profile::profile::ProfileData;
use sekai_profile::region::Region;
use sekai_profile::types::{CustomProfileCard, UserCustomProfileCard};
use sekai_profile::{Background, CustomProfileRenderer, RenderConfig, SdfExecutor};
use serde_json::Value;

use crate::fetch::AssetUrlLayout;

#[derive(Parser)]
#[command(
    name = "sekai-profile",
    version,
    about = "Render Project SEKAI custom profile cards"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Render one profile page to a PNG or JPEG file
    Render(RenderArgs),
    /// Build the text and shape atlases for every font family and shape
    Prepare(PrepareArgs),
    /// Answer render requests read as NDJSON from stdin
    Serve(ServeArgs),
}

#[derive(Args)]
struct MasterDataArgs {
    /// Directory of master data tables, read as `<DIR>/<table>.json`
    #[arg(
        long,
        value_name = "DIR",
        conflicts_with = "masterdata_url",
        required_unless_present = "masterdata_url"
    )]
    masterdata: Option<PathBuf>,
    /// URL prefix of master data tables, fetched as `<URL>/<table>.json`
    #[arg(long, value_name = "URL")]
    masterdata_url: Option<String>,
    /// Game server region: cn, jp, tw, kr or en
    #[arg(long, value_name = "REGION", default_value = "cn", value_parser = parse_region)]
    region: Region,
}

#[derive(Args)]
struct ResourceArgs {
    /// Directory of font files
    #[arg(long, value_name = "DIR", env = "SEKAI_PROFILE_FONT_DIR")]
    font_dir: Option<PathBuf>,
    /// Directory of PNG assets; a file's asset key is its relative path
    /// without `.png`
    #[arg(long, value_name = "DIR")]
    assets_dir: Option<PathBuf>,
    /// URL prefix assets missing from --assets-dir are fetched from
    #[arg(long, value_name = "URL")]
    assets_url: Option<String>,
    /// URL prefix of the engine's static assets (frames, icons, masks);
    /// they are fetched from --assets-url when this is not given
    #[arg(long, value_name = "URL", requires = "assets_url")]
    static_url: Option<String>,
    /// Path layout under the asset URLs: `flat` fetches `<key>.png`,
    /// `game-assets` follows the game's asset bundle directories
    #[arg(long, value_enum, value_name = "LAYOUT", default_value_t = AssetUrlLayout::Flat)]
    asset_url_layout: AssetUrlLayout,
}

#[derive(Args)]
struct RenderArgs {
    /// Profile API response (JSON). Supplies the player data and, without
    /// --card, the pages
    #[arg(long, value_name = "FILE", required_unless_present = "card")]
    profile: Option<PathBuf>,
    /// Card JSON: a `customProfileCard` object or a `userCustomProfileCards`
    /// array
    #[arg(long, value_name = "FILE")]
    card: Option<PathBuf>,
    /// Page to render, counted from 1 in the order the game shows them
    #[arg(long, value_name = "N", default_value_t = 1,
          value_parser = clap::value_parser!(u32).range(1..), conflicts_with = "seq")]
    page: u32,
    /// Render the page with this `seq` instead
    #[arg(long, value_name = "SEQ")]
    seq: Option<i32>,
    #[command(flatten)]
    masterdata: MasterDataArgs,
    #[command(flatten)]
    resources: ResourceArgs,
    /// Atlas directory. Glyphs and shapes the page needs are added to it
    /// before rendering unless --no-prepare is given
    #[arg(
        long,
        value_name = "DIR",
        env = "SEKAI_PROFILE_ATLAS_DIR",
        default_value = "atlases"
    )]
    atlas_dir: PathBuf,
    /// Render with the atlases as they are
    #[arg(long)]
    no_prepare: bool,
    /// Pre-decoded render-object store, by its manifest
    #[arg(long, value_name = "MANIFEST")]
    render_objects: Option<PathBuf>,
    /// Output file
    #[arg(short, long, value_name = "FILE")]
    output: PathBuf,
    /// Output format; follows the extension of --output when not given
    #[arg(long, value_enum)]
    format: Option<OutputFormat>,
    /// Keep the page background transparent (PNG only)
    #[arg(long)]
    transparent: bool,
    /// JPEG quality; ignored for PNG output
    #[arg(long, value_name = "1-100", default_value_t = 90,
          value_parser = clap::value_parser!(u8).range(1..=100))]
    quality: u8,
    /// SDF executor; `auto` uses AVX-512 when the CPU supports it, and
    /// `simd` fails on a CPU without AVX-512 F, BW and VBMI plus FMA
    #[arg(long, value_enum, default_value_t = Executor::Auto)]
    executor: Executor,
}

#[derive(Args)]
struct PrepareArgs {
    /// Atlas directory to fill
    #[arg(
        long,
        value_name = "DIR",
        env = "SEKAI_PROFILE_ATLAS_DIR",
        default_value = "atlases"
    )]
    atlas_dir: PathBuf,
    /// Cover only the text of these profiles (Profile API responses, JSON)
    /// instead of every character the fonts map
    #[arg(long = "for", value_name = "FILE")]
    for_profiles: Vec<PathBuf>,
    /// Rebuild atlases that already cover the request
    #[arg(long)]
    force: bool,
    #[command(flatten)]
    masterdata: MasterDataArgs,
    #[command(flatten)]
    resources: ResourceArgs,
}

#[derive(Args)]
struct ServeArgs {
    /// Atlas directory written by `prepare`
    #[arg(
        long,
        value_name = "DIR",
        env = "SEKAI_PROFILE_ATLAS_DIR",
        default_value = "atlases"
    )]
    atlas_dir: PathBuf,
    #[command(flatten)]
    masterdata: MasterDataArgs,
    #[command(flatten)]
    resources: ResourceArgs,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum OutputFormat {
    Png,
    Jpeg,
}

impl OutputFormat {
    fn from_extension(path: &Path) -> Option<Self> {
        let extension = path.extension()?.to_str()?.to_ascii_lowercase();
        match extension.as_str() {
            "png" => Some(Self::Png),
            "jpg" | "jpeg" => Some(Self::Jpeg),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum Executor {
    Auto,
    Simd,
    Scalar,
}

impl From<Executor> for SdfExecutor {
    fn from(executor: Executor) -> Self {
        match executor {
            Executor::Auto => Self::Auto,
            Executor::Simd => Self::Simd,
            Executor::Scalar => Self::Scalar,
        }
    }
}

fn parse_region(value: &str) -> Result<Region, String> {
    Region::from_str(value)
        .ok_or_else(|| format!("unknown region {value}; expected cn, jp, tw, kr or en"))
}

/// Which page of a card list to draw.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageSelection {
    /// Zero-based position in the order the game shows the pages.
    Index(usize),
    Seq(i32),
}

/// The card to draw: `card` when given, otherwise the profile's
/// `userCustomProfileCards`. A single `customProfileCard` object is drawn as
/// is; from a page list the selected page is drawn.
pub(crate) fn select_card(
    card: Option<Value>,
    profile: Option<&Value>,
    selection: PageSelection,
) -> Result<CustomProfileCard, String> {
    let value = match card {
        Some(card) => card,
        None => profile
            .and_then(|profile| profile.get("userCustomProfileCards"))
            .cloned()
            .ok_or("the profile has no userCustomProfileCards; pass --card")?,
    };
    if !value.is_array() {
        return serde_json::from_value(value)
            .map_err(|error| format!("parse customProfileCard failed: {error}"));
    }
    let pages: Vec<UserCustomProfileCard> = serde_json::from_value(value)
        .map_err(|error| format!("parse userCustomProfileCards failed: {error}"))?;
    let page = match selection {
        PageSelection::Index(index) => {
            sekai_profile::core::profile_source::profile_page(&pages, index).ok_or_else(|| {
                format!("there is no page {} among {} pages", index + 1, pages.len())
            })?
        }
        PageSelection::Seq(seq) => pages
            .iter()
            .find(|page| page.seq == seq)
            .ok_or_else(|| format!("there is no page with seq {seq}"))?,
    };
    Ok(page.custom_profile_card.clone())
}

fn read_json(path: &Path) -> Result<Value, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("read {} failed: {error}", path.display()))?;
    serde_json::from_str(&text).map_err(|error| format!("parse {} failed: {error}", path.display()))
}

/// Master data, assets and asset sources shared by every subcommand.
struct Resources {
    masterdata: Arc<JsonMasterDataProvider>,
    assets: Arc<AssetStore>,
    asset_urls: serve::AssetUrls,
    has_assets_dir: bool,
}

impl Resources {
    fn load(masterdata: &MasterDataArgs, resources: &ResourceArgs) -> Result<Self, String> {
        if let Some(font_dir) = &resources.font_dir {
            sekai_profile::set_font_dirs([font_dir]);
        }
        let provider = match (&masterdata.masterdata, &masterdata.masterdata_url) {
            (Some(dir), _) => JsonMasterDataProvider::from_dir(dir)?,
            (None, Some(url)) => {
                let mut provider = JsonMasterDataProvider::empty();
                let count = fetch::load_masterdata_url(&mut provider, url)?;
                tracing::info!(count, %url, "fetched master data");
                provider
            }
            (None, None) => return Err("pass --masterdata or --masterdata-url".into()),
        }
        .with_region(masterdata.region);
        let missing = provider.missing_tables();
        if !missing.is_empty() {
            tracing::warn!(
                ?missing,
                "master data tables are missing; elements that use them are not drawn"
            );
        }
        let assets = Arc::new(AssetStore::new(1024));
        if let Some(dir) = &resources.assets_dir {
            let count = assets.load_png_dir(dir)?;
            tracing::info!(count, dir = %dir.display(), "loaded assets");
        }
        Ok(Self {
            masterdata: Arc::new(provider),
            assets,
            asset_urls: serve::AssetUrls {
                dynamic: resources.assets_url.clone(),
                static_: resources.static_url.clone(),
                layout: resources.asset_url_layout,
            },
            has_assets_dir: resources.assets_dir.is_some(),
        })
    }

    fn renderer(&self) -> CustomProfileRenderer {
        CustomProfileRenderer::new(self.masterdata.clone()).with_assets(Arc::clone(&self.assets))
    }

    fn fetch(&self, keys: &[String]) {
        fetch_assets(&self.assets, &self.asset_urls, keys);
    }

    fn has_asset_source(&self) -> bool {
        self.asset_urls.dynamic.is_some() || self.has_assets_dir
    }

    /// Fills `dir` as `options` asks, reading shape sprites from the assets.
    fn prepare(
        &self,
        dir: &AtlasDir,
        mut options: PrepareOptions,
    ) -> Result<PrepareReport, String> {
        if !self.has_asset_source() {
            tracing::warn!("no asset source is given, so the shape atlas is not built");
            options.skip_shapes = true;
        }
        let keys: Vec<String> = self
            .masterdata
            .table("customProfileShapeResources")
            .map(|table| {
                table
                    .all()
                    .iter()
                    .filter_map(|row| {
                        serde_json::from_value::<sekai_profile::prepare::ShapeResource>(row.clone())
                            .ok()
                    })
                    .map(|resource| resource.asset_key())
                    .collect()
            })
            .unwrap_or_default();
        let shape_dir = dir.shape_dir().join("manifest.json");
        if !options.skip_shapes && (options.force || !shape_dir.is_file()) {
            self.fetch(&keys);
        }
        let report = prepare_atlas_dir(
            dir,
            &self.masterdata,
            |resource| {
                self.assets
                    .get(&resource.asset_key())
                    .map(|bytes| bytes.to_vec())
                    .ok_or_else(|| "sprite is not among the assets".to_string())
            },
            &options,
        )?;
        for (family, prepared) in &report.text {
            if *prepared == Prepared::Built {
                tracing::info!(%family, "built text atlas");
            }
        }
        if report.shape == Some(Prepared::Built) {
            tracing::info!("built shape atlas");
        }
        if !report.missing_fonts.is_empty() {
            tracing::warn!(
                families = ?report.missing_fonts,
                "no font file is installed for these families; cards that use them cannot be rendered"
            );
        }
        Ok(report)
    }
}

/// Fetches the assets of `keys` the store lacks from the asset URLs, when
/// they are configured.
pub(crate) fn fetch_assets(assets: &Arc<AssetStore>, urls: &serve::AssetUrls, keys: &[String]) {
    let Some(url) = &urls.dynamic else {
        return;
    };
    let wanted: Vec<String> = keys
        .iter()
        .filter(|key| !assets.contains(key))
        .cloned()
        .collect();
    if wanted.is_empty() {
        return;
    }
    let (fetched, failed) =
        fetch::load_assets_url(assets, &wanted, url, urls.static_.as_deref(), urls.layout);
    tracing::info!(fetched, failed, "fetched assets");
}

/// Asset keys `card` and `profile` draw that the store lacks.
pub(crate) fn missing_asset_keys(
    renderer: &CustomProfileRenderer,
    card: &CustomProfileCard,
    profile: Option<&ProfileData>,
    store: &AssetStore,
) -> Vec<String> {
    let md = renderer.masterdata();
    let mut keys = sekai_profile::asset_keys::missing_card_asset_keys_with_profile(
        card,
        profile,
        &md,
        |key| store.contains(key),
    );
    if let Some(profile) = profile {
        keys.extend(sekai_profile::asset_keys::missing_profile_asset_keys(
            profile,
            &md,
            |key| store.contains(key),
        ));
    }
    keys.sort();
    keys.dedup();
    keys
}

/// Renders `card` and encodes it.
pub(crate) fn render_page(
    renderer: &CustomProfileRenderer,
    card: &CustomProfileCard,
    profile: Option<&ProfileData>,
    config: &RenderConfig,
    format: OutputFormat,
    quality: u8,
) -> Result<Vec<u8>, String> {
    let page = renderer
        .render(card, profile, config)
        .map_err(|error| error.to_string())?;
    match format {
        OutputFormat::Png => page.to_png(),
        OutputFormat::Jpeg => {
            if config.background == Background::Transparent {
                return Err("JPEG has no transparency; write a PNG".into());
            }
            encode_jpeg(&page, quality)
        }
    }
}

#[cfg(feature = "jpeg-turbo")]
fn encode_jpeg(page: &sekai_profile::RenderedPage, quality: u8) -> Result<Vec<u8>, String> {
    page.to_jpeg(quality)
}

#[cfg(not(feature = "jpeg-turbo"))]
fn encode_jpeg(_page: &sekai_profile::RenderedPage, _quality: u8) -> Result<Vec<u8>, String> {
    Err("this build has no JPEG encoder; write a PNG or build with the jpeg-turbo feature".into())
}

fn render(args: RenderArgs) -> Result<(), String> {
    let format = match args.format {
        Some(format) => format,
        None => OutputFormat::from_extension(&args.output).ok_or_else(|| {
            format!(
                "cannot tell the format of {} from its extension; pass --format",
                args.output.display()
            )
        })?,
    };
    if args.transparent && format == OutputFormat::Jpeg {
        return Err("JPEG has no transparency; write a PNG".into());
    }
    let resources = Resources::load(&args.masterdata, &args.resources)?;
    let profile_json = args.profile.as_deref().map(read_json).transpose()?;
    let card_json = args.card.as_deref().map(read_json).transpose()?;
    let selection = match args.seq {
        Some(seq) => PageSelection::Seq(seq),
        None => PageSelection::Index(args.page as usize - 1),
    };
    let card = select_card(card_json, profile_json.as_ref(), selection)?;
    let profile = profile_json.as_ref().map(ProfileData::from_json);

    let mut renderer = resources.renderer();
    resources.fetch(&missing_asset_keys(
        &renderer,
        &card,
        profile.as_ref(),
        &resources.assets,
    ));
    let dir = AtlasDir::new(&args.atlas_dir);
    if !args.no_prepare {
        let codepoints = renderer
            .text_codepoints(&card, profile.as_ref())
            .map_err(|error| error.to_string())?;
        resources.prepare(
            &dir,
            PrepareOptions {
                codepoints: Some(codepoints),
                ..PrepareOptions::default()
            },
        )?;
    }
    renderer = renderer.with_atlas_dir(dir.root())?;
    if let Some(manifest) = &args.render_objects {
        let store = sekai_profile::render_object::MappedRenderObjectStore::open(manifest)
            .map_err(|error| format!("open {} failed: {error}", manifest.display()))?;
        renderer = renderer.with_render_object_store(Arc::new(store));
    }

    for warning in renderer.validate_card(&card) {
        tracing::warn!("{warning}");
    }
    let missing = missing_asset_keys(&renderer, &card, profile.as_ref(), &resources.assets);
    if !missing.is_empty() {
        tracing::warn!(count = missing.len(), keys = ?missing, "assets are missing and are drawn transparent");
    }
    let config = RenderConfig {
        executor: args.executor.into(),
        background: if args.transparent {
            Background::Transparent
        } else {
            Background::White
        },
        ..RenderConfig::default()
    };
    let bytes = render_page(
        &renderer,
        &card,
        profile.as_ref(),
        &config,
        format,
        args.quality,
    )?;
    std::fs::write(&args.output, &bytes)
        .map_err(|error| format!("write {} failed: {error}", args.output.display()))?;
    tracing::info!(bytes = bytes.len(), path = %args.output.display(), "rendered");
    Ok(())
}

fn prepare(args: PrepareArgs) -> Result<(), String> {
    let resources = Resources::load(&args.masterdata, &args.resources)?;
    let dir = AtlasDir::new(&args.atlas_dir);
    let codepoints = if args.for_profiles.is_empty() {
        None
    } else {
        let renderer = resources.renderer();
        let mut demand =
            std::collections::BTreeMap::<String, std::collections::BTreeSet<u32>>::new();
        for path in &args.for_profiles {
            let json = read_json(path)?;
            let profile = ProfileData::from_json(&json);
            let pages: Vec<UserCustomProfileCard> = json
                .get("userCustomProfileCards")
                .map(|pages| serde_json::from_value(pages.clone()))
                .transpose()
                .map_err(|error| format!("{}: {error}", path.display()))?
                .unwrap_or_default();
            for page in &pages {
                let card = &page.custom_profile_card;
                let needed = renderer
                    .text_codepoints(card, Some(&profile))
                    .map_err(|error| format!("{}: {error}", path.display()))?;
                for (family, codepoints) in needed {
                    demand.entry(family).or_default().extend(codepoints);
                }
            }
        }
        Some(demand)
    };
    let report = resources.prepare(
        &dir,
        PrepareOptions {
            codepoints,
            force: args.force,
            ..PrepareOptions::default()
        },
    )?;
    let built = report
        .text
        .values()
        .filter(|prepared| **prepared == Prepared::Built)
        .count();
    println!(
        "text atlases: {built} built, {} kept; shape atlas: {}; missing fonts: {}",
        report.text.len() - built,
        match report.shape {
            Some(Prepared::Built) => "built",
            Some(Prepared::Kept) => "kept",
            None => "not built",
        },
        if report.missing_fonts.is_empty() {
            "none".to_string()
        } else {
            report.missing_fonts.join(", ")
        }
    );
    Ok(())
}

fn serve(args: ServeArgs) -> Result<ExitCode, String> {
    let resources = Resources::load(&args.masterdata, &args.resources)?;
    let renderer = resources.renderer().with_atlas_dir(&args.atlas_dir)?;
    Ok(serve::run(
        renderer,
        resources.assets,
        resources.asset_urls,
        args.masterdata.region,
    ))
}

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    let result = match Cli::parse().command {
        Command::Render(args) => render(args).map(|()| ExitCode::SUCCESS),
        Command::Prepare(args) => prepare(args).map(|()| ExitCode::SUCCESS),
        Command::Serve(args) => serve(args),
    };
    match result {
        Ok(code) => code,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
    use serde_json::json;

    #[test]
    fn the_command_line_definition_is_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn the_output_format_follows_the_file_extension() {
        assert_eq!(
            OutputFormat::from_extension(Path::new("a.PNG")),
            Some(OutputFormat::Png)
        );
        assert_eq!(
            OutputFormat::from_extension(Path::new("a.jpeg")),
            Some(OutputFormat::Jpeg)
        );
        assert_eq!(
            OutputFormat::from_extension(Path::new("a.jpg")),
            Some(OutputFormat::Jpeg)
        );
        assert_eq!(OutputFormat::from_extension(Path::new("a.webp")), None);
        assert_eq!(OutputFormat::from_extension(Path::new("a")), None);
    }

    fn page(id: i32, seq: i32) -> Value {
        json!({
            "customProfileCard": {},
            "customProfileCardId": id,
            "customProfileId": 1,
            "seq": seq,
        })
    }

    fn card_id(card: &CustomProfileCard) -> Value {
        serde_json::to_value(card).expect("card")
    }

    #[test]
    fn pages_come_from_the_profile_in_the_order_the_game_shows_them() {
        let profile = json!({ "userCustomProfileCards": [page(10, 7), page(11, 3)] });
        let first = select_card(None, Some(&profile), PageSelection::Index(0)).expect("first");
        let by_seq = select_card(None, Some(&profile), PageSelection::Seq(3)).expect("seq 3");
        assert_eq!(card_id(&first), card_id(&by_seq));
        assert!(select_card(None, Some(&profile), PageSelection::Index(2)).is_err());
        assert!(select_card(None, Some(&profile), PageSelection::Seq(5)).is_err());
    }

    #[test]
    fn a_card_argument_takes_precedence_over_the_profile_pages() {
        let profile = json!({ "userCustomProfileCards": "not pages" });
        let card = select_card(Some(json!({})), Some(&profile), PageSelection::Index(0))
            .expect("the card argument");
        assert_eq!(
            card,
            serde_json::from_value::<CustomProfileCard>(json!({})).expect("empty card")
        );
        assert!(select_card(None, Some(&profile), PageSelection::Index(0)).is_err());
        assert!(select_card(None, None, PageSelection::Index(0)).is_err());
    }

    #[test]
    fn render_arguments_parse_with_a_profile_alone() {
        let cli = Cli::try_parse_from([
            "sekai-profile",
            "render",
            "--profile",
            "profile.json",
            "--masterdata",
            "masterdata",
            "-o",
            "card.png",
        ])
        .expect("parse");
        let Command::Render(args) = cli.command else {
            panic!("render expected");
        };
        assert_eq!(args.page, 1);
        assert_eq!(args.executor, Executor::Auto);
        assert!(Cli::try_parse_from([
            "sekai-profile",
            "render",
            "--masterdata",
            "m",
            "-o",
            "a.png"
        ])
        .is_err());
        assert!(Cli::try_parse_from([
            "sekai-profile",
            "render",
            "--card",
            "c.json",
            "--masterdata",
            "m",
            "--masterdata-url",
            "https://example.invalid",
            "-o",
            "a.png",
        ])
        .is_err());
    }
}
