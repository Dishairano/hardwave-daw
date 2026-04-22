//! Marketplace catalog model — data shape for the in-app store:
//! sample packs, preset packs, and plugins, plus user accounts,
//! purchase history, creator revenue, and ratings.
//!
//! This is the data layer only; the network layer + payment flow
//! plug in at a higher tier. The catalog can be exercised offline
//! from fixtures so UI work can proceed without a backend.

use serde::{Deserialize, Serialize};

/// Top-level categorization — drives the store's left-nav tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StoreSection {
    SamplePacks,
    Presets,
    Plugins,
}

/// Preset instrument — used to filter "Browse presets by instrument".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PresetInstrument {
    Subtractive,
    Wavetable,
    Fm,
    Sampler,
    DrumMachine,
    DrumSynth,
    Generic,
}

/// One catalog entry. `kind` carries the section-specific payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreItem {
    pub id: String,
    pub title: String,
    pub creator_id: String,
    pub price_cents: u32,
    pub preview_url: Option<String>,
    pub install_path: String,
    pub rating_count: u32,
    pub rating_sum: u32,
    pub kind: StoreItemKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StoreItemKind {
    SamplePack {
        sample_count: u32,
        style_tags: Vec<String>,
    },
    Preset {
        instrument: PresetInstrument,
        preset_count: u32,
    },
    Plugin {
        vendor: String,
        format: PluginFormat,
        version: String,
    },
}

impl StoreItem {
    pub fn section(&self) -> StoreSection {
        match &self.kind {
            StoreItemKind::SamplePack { .. } => StoreSection::SamplePacks,
            StoreItemKind::Preset { .. } => StoreSection::Presets,
            StoreItemKind::Plugin { .. } => StoreSection::Plugins,
        }
    }

    pub fn average_rating(&self) -> Option<f32> {
        if self.rating_count == 0 {
            None
        } else {
            Some(self.rating_sum as f32 / self.rating_count as f32)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PluginFormat {
    Vst3,
    Clap,
    Native,
}

/// Full catalog — an owner struct the UI queries for browse /
/// search / filter. In-memory so the store page renders instantly;
/// the backend syncs this via a periodic pull.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Catalog {
    pub items: Vec<StoreItem>,
}

impl Catalog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn by_section(&self, section: StoreSection) -> Vec<&StoreItem> {
        self.items
            .iter()
            .filter(|i| i.section() == section)
            .collect()
    }

    pub fn presets_by_instrument(&self, instrument: PresetInstrument) -> Vec<&StoreItem> {
        self.items
            .iter()
            .filter(|i| matches!(&i.kind, StoreItemKind::Preset { instrument: inst, .. } if *inst == instrument))
            .collect()
    }

    pub fn find(&self, id: &str) -> Option<&StoreItem> {
        self.items.iter().find(|i| i.id == id)
    }

    pub fn search(&self, query: &str) -> Vec<&StoreItem> {
        let q = query.to_lowercase();
        self.items
            .iter()
            .filter(|i| i.title.to_lowercase().contains(&q))
            .collect()
    }

    pub fn top_rated(&self, section: StoreSection, limit: usize) -> Vec<&StoreItem> {
        let mut items: Vec<&StoreItem> = self.by_section(section);
        items.sort_by(|a, b| {
            b.average_rating()
                .unwrap_or(0.0)
                .partial_cmp(&a.average_rating().unwrap_or(0.0))
                .unwrap()
        });
        items.into_iter().take(limit).collect()
    }
}

/// A user's purchase history. Per-item record with timestamp.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Purchase {
    pub item_id: String,
    pub price_cents_paid: u32,
    pub purchase_timestamp_unix: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UserAccount {
    pub user_id: String,
    pub display_name: String,
    pub email: String,
    pub purchases: Vec<Purchase>,
}

impl UserAccount {
    pub fn owns(&self, item_id: &str) -> bool {
        self.purchases.iter().any(|p| p.item_id == item_id)
    }

    pub fn add_purchase(&mut self, purchase: Purchase) {
        if !self.owns(&purchase.item_id) {
            self.purchases.push(purchase);
        }
    }

    pub fn total_spent_cents(&self) -> u64 {
        self.purchases
            .iter()
            .map(|p| p.price_cents_paid as u64)
            .sum()
    }
}

/// Creator-side revenue record. `revenue_cents` is the amount the
/// creator actually receives after the platform cut.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CreatorRevenue {
    pub creator_id: String,
    pub platform_fee_bps: u16,
    pub total_sales_cents: u64,
    pub platform_cut_cents: u64,
    pub creator_cents: u64,
}

impl CreatorRevenue {
    /// Record a sale for this creator — splits platform_fee_bps
    /// basis points to the platform and the rest to the creator.
    pub fn record_sale(&mut self, price_cents: u32) {
        self.total_sales_cents += price_cents as u64;
        let platform = (price_cents as u64) * (self.platform_fee_bps as u64) / 10_000;
        let creator = price_cents as u64 - platform;
        self.platform_cut_cents += platform;
        self.creator_cents += creator;
    }
}

/// A rating + optional written review for a store item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Review {
    pub user_id: String,
    pub item_id: String,
    pub stars: u8, // 1..=5
    pub body: Option<String>,
    pub timestamp_unix: i64,
}

impl Review {
    pub fn new(user_id: impl Into<String>, item_id: impl Into<String>, stars: u8) -> Self {
        Self {
            user_id: user_id.into(),
            item_id: item_id.into(),
            stars: stars.clamp(1, 5),
            body: None,
            timestamp_unix: 0,
        }
    }
}

/// Register a review against a catalog item — adds the user's
/// stars to the item's aggregate rating counters. Replaces any
/// existing review by the same user on the same item.
pub fn register_review(
    catalog: &mut Catalog,
    reviews: &mut Vec<Review>,
    new_review: Review,
) -> bool {
    let item = catalog
        .items
        .iter_mut()
        .find(|i| i.id == new_review.item_id);
    let Some(item) = item else {
        return false;
    };
    // Remove any prior review by this user on this item, updating
    // the item's aggregate counters.
    let prior = reviews
        .iter()
        .position(|r| r.user_id == new_review.user_id && r.item_id == new_review.item_id);
    if let Some(idx) = prior {
        let old = reviews.remove(idx);
        item.rating_sum = item.rating_sum.saturating_sub(old.stars as u32);
        item.rating_count = item.rating_count.saturating_sub(1);
    }
    item.rating_sum += new_review.stars as u32;
    item.rating_count += 1;
    reviews.push(new_review);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_pack(id: &str, title: &str, price: u32) -> StoreItem {
        StoreItem {
            id: id.into(),
            title: title.into(),
            creator_id: "creator-1".into(),
            price_cents: price,
            preview_url: Some("https://example/preview.mp3".into()),
            install_path: format!("~/Library/Hardwave/{}", id),
            rating_count: 0,
            rating_sum: 0,
            kind: StoreItemKind::SamplePack {
                sample_count: 100,
                style_tags: vec!["trap".into()],
            },
        }
    }

    fn preset_pack(id: &str, instrument: PresetInstrument) -> StoreItem {
        StoreItem {
            id: id.into(),
            title: format!("{} {:?} Pack", id, instrument),
            creator_id: "creator-2".into(),
            price_cents: 1_999,
            preview_url: None,
            install_path: format!("~/Library/Hardwave/{}", id),
            rating_count: 0,
            rating_sum: 0,
            kind: StoreItemKind::Preset {
                instrument,
                preset_count: 32,
            },
        }
    }

    fn plugin(id: &str, fmt: PluginFormat) -> StoreItem {
        StoreItem {
            id: id.into(),
            title: format!("Plugin {}", id),
            creator_id: "creator-3".into(),
            price_cents: 9_999,
            preview_url: None,
            install_path: format!("~/VST/{}", id),
            rating_count: 0,
            rating_sum: 0,
            kind: StoreItemKind::Plugin {
                vendor: "Hardwave Labs".into(),
                format: fmt,
                version: "1.0.0".into(),
            },
        }
    }

    #[test]
    fn by_section_filters_correctly() {
        let mut cat = Catalog::new();
        cat.items.push(sample_pack("sp1", "Trap Kit", 999));
        cat.items.push(preset_pack("ps1", PresetInstrument::Fm));
        cat.items.push(plugin("pg1", PluginFormat::Vst3));
        assert_eq!(cat.by_section(StoreSection::SamplePacks).len(), 1);
        assert_eq!(cat.by_section(StoreSection::Presets).len(), 1);
        assert_eq!(cat.by_section(StoreSection::Plugins).len(), 1);
    }

    #[test]
    fn presets_by_instrument_filters() {
        let mut cat = Catalog::new();
        cat.items.push(preset_pack("fm1", PresetInstrument::Fm));
        cat.items
            .push(preset_pack("ws1", PresetInstrument::Wavetable));
        assert_eq!(cat.presets_by_instrument(PresetInstrument::Fm).len(), 1);
        assert_eq!(
            cat.presets_by_instrument(PresetInstrument::Wavetable).len(),
            1
        );
        assert_eq!(
            cat.presets_by_instrument(PresetInstrument::Sampler).len(),
            0
        );
    }

    #[test]
    fn search_matches_case_insensitive() {
        let mut cat = Catalog::new();
        cat.items.push(sample_pack("sp1", "Trap Sounds 2026", 999));
        cat.items.push(sample_pack("sp2", "House Essentials", 999));
        assert_eq!(cat.search("trap").len(), 1);
        assert_eq!(cat.search("TRAP").len(), 1);
        assert_eq!(cat.search("dubstep").len(), 0);
    }

    #[test]
    fn user_account_tracks_purchase_history_and_idempotency() {
        let mut user = UserAccount {
            user_id: "u1".into(),
            display_name: "Dex".into(),
            email: "d@x.com".into(),
            purchases: Vec::new(),
        };
        user.add_purchase(Purchase {
            item_id: "sp1".into(),
            price_cents_paid: 999,
            purchase_timestamp_unix: 1,
        });
        user.add_purchase(Purchase {
            item_id: "sp1".into(),
            price_cents_paid: 999,
            purchase_timestamp_unix: 2,
        });
        assert_eq!(user.purchases.len(), 1);
        assert!(user.owns("sp1"));
        assert_eq!(user.total_spent_cents(), 999);
    }

    #[test]
    fn creator_revenue_splits_platform_and_creator() {
        let mut rev = CreatorRevenue {
            creator_id: "c1".into(),
            platform_fee_bps: 3_000, // 30% platform cut
            total_sales_cents: 0,
            platform_cut_cents: 0,
            creator_cents: 0,
        };
        rev.record_sale(1_000);
        assert_eq!(rev.total_sales_cents, 1_000);
        assert_eq!(rev.platform_cut_cents, 300);
        assert_eq!(rev.creator_cents, 700);
    }

    #[test]
    fn review_registration_updates_average_rating() {
        let mut cat = Catalog::new();
        cat.items.push(sample_pack("sp1", "Pack", 999));
        let mut reviews = Vec::new();
        let ok = register_review(&mut cat, &mut reviews, Review::new("u1", "sp1", 5));
        assert!(ok);
        assert_eq!(cat.find("sp1").unwrap().rating_count, 1);
        assert_eq!(cat.find("sp1").unwrap().average_rating(), Some(5.0));
        // Second user contributes lower rating.
        register_review(&mut cat, &mut reviews, Review::new("u2", "sp1", 3));
        assert_eq!(cat.find("sp1").unwrap().rating_count, 2);
        assert!((cat.find("sp1").unwrap().average_rating().unwrap() - 4.0).abs() < 1e-3);
    }

    #[test]
    fn re_reviewing_replaces_prior_entry() {
        let mut cat = Catalog::new();
        cat.items.push(sample_pack("sp1", "Pack", 999));
        let mut reviews = Vec::new();
        register_review(&mut cat, &mut reviews, Review::new("u1", "sp1", 5));
        register_review(&mut cat, &mut reviews, Review::new("u1", "sp1", 2));
        assert_eq!(cat.find("sp1").unwrap().rating_count, 1);
        assert_eq!(cat.find("sp1").unwrap().average_rating(), Some(2.0));
        assert_eq!(reviews.len(), 1);
    }

    #[test]
    fn top_rated_returns_sorted_limited_list() {
        let mut cat = Catalog::new();
        let mut a = sample_pack("a", "A", 0);
        a.rating_count = 1;
        a.rating_sum = 3;
        let mut b = sample_pack("b", "B", 0);
        b.rating_count = 1;
        b.rating_sum = 5;
        cat.items.push(a);
        cat.items.push(b);
        let top = cat.top_rated(StoreSection::SamplePacks, 1);
        assert_eq!(top.len(), 1);
        assert_eq!(top[0].id, "b");
    }
}
