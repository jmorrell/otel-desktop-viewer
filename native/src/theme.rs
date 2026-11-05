use gpui_kit::*;
#[derive(Clone, Copy)]
pub(crate) struct Palette {
    pub canvas: u32,
    pub panel: u32,
    pub chrome: u32,
    pub text: u32,
    pub subtle: u32,
    pub accent: u32,
    pub selection: u32,
    pub error: u32,
}
static SEASON: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(2);
static INK: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(usize::MAX);
pub(crate) fn selection(season: usize, ink: usize) {
    SEASON.store(season, std::sync::atomic::Ordering::Relaxed);
    INK.store(ink, std::sync::atomic::Ordering::Relaxed);
}
pub(crate) fn ink_catalogue() -> &'static serde_json::Value {
    static INKS: std::sync::OnceLock<serde_json::Value> = std::sync::OnceLock::new();
    INKS.get_or_init(|| {
        serde_json::from_str(include_str!("../assets/graph-inks.json")).expect("bundled graph inks")
    })
}
pub(crate) fn ink_names() -> Vec<String> {
    ink_catalogue()["inks"]
        .as_object()
        .expect("ink catalogue")
        .keys()
        .cloned()
        .collect()
}
pub(crate) fn ink_shades(index: usize) -> [u32; 3] {
    let names = ink_names();
    let index = if index == usize::MAX {
        names.iter().position(|n| n == "to-ro").unwrap_or(0)
    } else {
        index % names.len()
    };
    let shades = &ink_catalogue()["inks"][&names[index]];
    [6, 3, 0].map(|shade| {
        u32::from_str_radix(
            shades[shade]
                .as_str()
                .unwrap_or("#888888")
                .trim_start_matches('#'),
            16,
        )
        .unwrap_or(0x888888)
    })
}
pub(crate) fn palette(dark: bool) -> Palette {
    static THEMES: std::sync::OnceLock<serde_json::Value> = std::sync::OnceLock::new();
    let themes = THEMES.get_or_init(|| {
        serde_json::from_str(include_str!("../assets/madoromi.json"))
            .expect("bundled Madoromi themes")
    });
    let season =
        ["haru", "natsu", "aki", "fuyu"][SEASON.load(std::sync::atomic::Ordering::Relaxed) % 4];
    static INTERFACE: std::sync::OnceLock<serde_json::Value> = std::sync::OnceLock::new();
    let interface = INTERFACE.get_or_init(|| {
        serde_json::from_str(include_str!("../assets/interface-palette.json"))
            .expect("bundled interface palette")
    });
    let name = format!("madoromi-{season}-{}", if dark { "yoru" } else { "hiru" });
    let c = &themes[&name]["colors"];
    let override_colors = &interface[&name]["colors"];
    let get = |key: &str| {
        u32::from_str_radix(
            override_colors[key]
                .as_str()
                .or_else(|| c[key].as_str())
                .unwrap_or("#000000")
                .trim_start_matches('#'),
            16,
        )
        .unwrap_or(0)
    };
    Palette {
        canvas: get("canvas"),
        panel: get("panel"),
        chrome: get("chrome"),
        text: get("text"),
        subtle: get("subtle"),
        accent: if INK.load(std::sync::atomic::Ordering::Relaxed) == usize::MAX {
            get("primary")
        } else {
            readable_ink(
                ink_shades(INK.load(std::sync::atomic::Ordering::Relaxed))[1],
                get("canvas"),
                dark,
            )
        },
        selection: get("highlightMedium"),
        error: get("error"),
    }
}
pub(crate) fn color(value: u32) -> Hsla {
    rgb(value).into()
}

pub(crate) fn apply(dark: bool, cx: &mut App) {
    use gpui_kit::component::{Theme, ThemeMode};
    Theme::change(
        if dark {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        },
        None,
        cx,
    );
    let p = palette(dark);
    Theme::update(cx, |theme| {
        theme.font_family = "Atkinson Hyperlegible Next".into();
        theme.background = color(p.canvas);
        theme.foreground = color(p.text);
        theme.secondary_foreground = color(p.text);
        theme.secondary = color(p.panel);
        theme.secondary_hover = color(p.selection);
        theme.secondary_active = color(p.selection);
        theme.button_secondary_hover = color(p.selection);
        theme.button_secondary_active = color(p.selection);
        theme.button_secondary_foreground = color(p.text);
        theme.primary = color(p.accent);
        theme.border = color(p.chrome);
        theme.button = color(p.panel);
        theme.button_foreground = color(p.text);
        theme.button_hover = color(p.selection);
        theme.button_active = color(p.selection);
        theme.muted = color(p.chrome);
        theme.muted_foreground = color(p.subtle);
        theme.accent = color(p.selection);
        theme.accent_foreground = color(p.text);
        theme.selection = color(p.selection);
        theme.input = color(p.panel);
        theme.caret = color(p.accent);
    });
}

/// Start from the web viewer's exact identity hash and shared bright-ink ranking.
/// The automatic colour stays stable across seasons and Day/Night.
pub(crate) fn service_color(identity: &str, _dark: bool) -> u32 {
    static RANKING: std::sync::OnceLock<Vec<u32>> = std::sync::OnceLock::new();
    let colors = RANKING.get_or_init(|| {
        let ranking: serde_json::Value =
            serde_json::from_str(include_str!("../assets/bright-ink-slots.json"))
                .expect("bundled bright ink ranking");
        ranking["slots"]
            .as_array()
            .expect("ink slots")
            .iter()
            .map(|slot| {
                let (ink, shade) = slot
                    .as_str()
                    .expect("ink id")
                    .rsplit_once(':')
                    .expect("ink shade");
                let shade: usize = shade.parse().expect("shade number");
                let hex = ink_catalogue()["inks"][ink][shade - 1]
                    .as_str()
                    .expect("graph color");
                u32::from_str_radix(hex.trim_start_matches('#'), 16).expect("graph color hex")
            })
            .collect()
    });
    colors[color_identity_hash(identity) as usize % colors.len()]
}
fn color_identity_hash(identity: &str) -> u32 {
    // JavaScript charCodeAt traverses UTF-16 code units, including surrogate pairs.
    let mut hash = identity.encode_utf16().fold(2166136261u32, |hash, ch| {
        (hash ^ u32::from(ch)).wrapping_mul(16777619)
    });
    hash ^= hash >> 16;
    hash = hash.wrapping_mul(0x85ebca6b);
    hash ^= hash >> 13;
    hash = hash.wrapping_mul(0xc2b2ae35);
    hash ^ (hash >> 16)
}

#[cfg(test)]
mod identity_tests {
    #[test]
    fn matches_web_identity_hash_for_ascii_and_surrogate_pairs() {
        for (identity, expected) in [
            ("api-gateway", 3115499377),
            ("checkout", 1259066387),
            ("payments", 340755266),
            ("🚀 telemetry", 1153140253),
            ("", 2872998923),
        ] {
            assert_eq!(super::color_identity_hash(identity), expected);
            assert_eq!(
                super::service_color(identity, false),
                super::service_color(identity, true)
            );
        }
    }
}

fn readable_ink(mut ink: u32, background: u32, dark: bool) -> u32 {
    let luminance = |rgb: u32| {
        let channel = |shift| {
            let c = ((rgb >> shift) & 255u32) as f64 / 255.;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0)
    };
    for _ in 0..64 {
        let (a, b) = (luminance(ink), luminance(background));
        if (a.max(b) + 0.05) / (a.min(b) + 0.05) >= 4.5 {
            break;
        }
        let next = |shift| {
            let c = ((ink >> shift) & 255u32) as u32;
            if dark {
                (c + 8).min(255)
            } else {
                c.saturating_sub(8)
            }
        };
        ink = (next(16) << 16) | (next(8) << 8) | next(0);
    }
    ink
}
