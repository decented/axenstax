//! The other games from the same makers, listed on the lobby's "More to play"
//! page (native only: the web taster carries no external links).
//!
//! A fixed, built-in list of our own three published Android games: no remote
//! list, nothing player-run, nothing submitted. The cards only open the
//! website's More games page, which follows each game's signed release and
//! carries the download.
//!
//! BRIDGE: static cards that open a web page -- replace when the lobby should
//! show each game's newest version. That is Option B in
//! `docs/superpowers/specs/2026-11-08-home-page-overhaul.md` section 7 (a read of each game's
//! signed release notice through nostr-release-kit, on the player's own
//! relays; the package, channel, release key and APK certificate pins belong
//! with that step, not here).

/// The website page that has the downloads. Cards open this, plus an anchor.
pub(crate) const PAGE_URL: &str = "https://axenstax.com/more-games";

/// One listed game.
pub(crate) struct MoreGame {
    pub name: &'static str,
    /// The game's own one-line blurb (kept under 60 bytes, as in the games).
    pub blurb: &'static str,
    /// The anchor of this game's card on the website page.
    pub anchor: &'static str,
    /// A 192x192 PNG, copied from the game's own repository.
    pub icon_png: &'static [u8],
}

impl MoreGame {
    /// Where this game's card opens: the website page, at this game.
    pub fn url(&self) -> String {
        format!("{PAGE_URL}#{}", self.anchor)
    }
}

pub(crate) const MORE_GAMES: [MoreGame; 3] = [
    MoreGame {
        name: "Stampede",
        blurb: "Laser-eyed ostrich racing, arcade style.",
        anchor: "stampede",
        icon_png: include_bytes!("../assets/games/stampede.png"),
    },
    MoreGame {
        name: "Twang Thang",
        blurb: "A neon energy-band party game.",
        anchor: "twang-thang",
        icon_png: include_bytes!("../assets/games/twangthang.png"),
    },
    MoreGame {
        name: "Cluster Flock",
        blurb: "Three-lane ostrich farm run, with ghost saboteurs.",
        anchor: "cluster-flock",
        icon_png: include_bytes!("../assets/games/clusterflock.png"),
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_games_with_names_short_blurbs_and_192_square_icons() {
        assert_eq!(MORE_GAMES.len(), 3);
        let names: Vec<_> = MORE_GAMES.iter().map(|g| g.name).collect();
        assert_eq!(names, ["Stampede", "Twang Thang", "Cluster Flock"]);
        for g in &MORE_GAMES {
            assert!(!g.name.is_empty() && !g.blurb.is_empty(), "{} has no text", g.name);
            assert!(g.blurb.len() < 60, "{:?} is {} bytes", g.blurb, g.blurb.len());
            let img = image::load_from_memory(g.icon_png).unwrap_or_else(|e| panic!("{} icon: {e}", g.name));
            assert_eq!((img.width(), img.height()), (192, 192), "{} icon size", g.name);
        }
    }

    #[test]
    fn every_card_opens_the_website_page_at_its_own_anchor() {
        assert_eq!(PAGE_URL, "https://axenstax.com/more-games");
        let urls: Vec<_> = MORE_GAMES.iter().map(MoreGame::url).collect();
        assert_eq!(
            urls,
            [
                "https://axenstax.com/more-games#stampede",
                "https://axenstax.com/more-games#twang-thang",
                "https://axenstax.com/more-games#cluster-flock",
            ]
        );
    }

    #[test]
    fn the_anchors_are_the_ones_the_website_page_has() {
        let page = include_str!("../../../tools/sites/marketing/app.py");
        // The page's ids are `name.lower().replace(" ", "-")`.
        assert!(page.contains(r#"slug=g["name"].lower().replace(" ", "-")"#));
        for g in &MORE_GAMES {
            assert_eq!(g.anchor, g.name.to_lowercase().replace(' ', "-"));
        }
    }

    #[test]
    fn the_copy_has_none_of_the_words_the_lobby_keeps_out() {
        for g in &MORE_GAMES {
            let all = format!("{} {}", g.name, g.blurb).to_lowercase();
            for word in ["friend", "online", "social", "earn", "sats", "store", "bitcoin", "chat"] {
                assert!(!all.contains(word), "{:?} contains {word:?}", all);
            }
        }
    }
}
