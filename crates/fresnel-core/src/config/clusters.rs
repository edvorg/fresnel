//! The options of each cluster (`[clusters."name"]`) and the cluster groups (`[groups]`).
//!
//! A cluster is a kubeconfig context, so the file names it by the name of its context. A
//! name that no kubeconfig file has now is no problem: a file can be absent for one run.

/// The confirm rule of a cluster: what the dialog of a change asks for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ConfirmRule {
    /// A dialog lists each target, and one click confirms (plan section 11.2).
    #[default]
    Dialog,
    /// The confirm button waits until the user typed the name of the cluster: a cluster of
    /// production gets one more step before a change.
    TypeName,
}

impl ConfirmRule {
    /// The rule of a text of the file. `None` for a text that names no rule.
    pub fn named(text: &str) -> Option<Self> {
        match text {
            "dialog" => Some(Self::Dialog),
            "type-name" => Some(Self::TypeName),
            _ => None,
        }
    }
}

/// A color: in the file, `"#3fb950"`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb(pub [u8; 3]);

impl Rgb {
    /// Reads `#rrggbb`. `None` for any other text.
    pub fn parse(text: &str) -> Option<Self> {
        let hex = text.strip_prefix('#').filter(|hex| hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit()))?;
        let byte = |at: usize| hex.get(at..at + 2).and_then(|pair| u8::from_str_radix(pair, 16).ok());
        Some(Self([byte(0)?, byte(2)?, byte(4)?]))
    }
}

impl std::fmt::Display for Rgb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let [red, green, blue] = self.0;
        write!(f, "#{red:02x}{green:02x}{blue:02x}")
    }
}

/// The options of one cluster. A field that the file does not set is `None`, so the
/// default of the group `[safety]` and of the app applies.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClusterOptions {
    /// The short name of the cluster in the window, in place of the name of its context.
    pub alias: Option<String>,
    /// The color of the cluster in place of the color of its place.
    pub color: Option<Rgb>,
    /// True when the app makes no change in the cluster: each write action is off.
    pub read_only: Option<bool>,
    /// The namespace of the filter at the start, and of a new Secret.
    pub default_namespace: Option<String>,
    /// The namespaces at the top of the namespace filter.
    pub favorite_namespaces: Vec<String>,
    pub confirm: Option<ConfirmRule>,
}

/// The option keys of a cluster, in the sequence of the settings view.
pub const OPTION_KEYS: [&str; 6] =
    ["alias", "color", "read_only", "default_namespace", "favorite_namespaces", "confirm"];

/// The longest alias. A longer one would push the other clusters out of the top bar.
pub const ALIAS_LIMIT: usize = 40;
/// The most favorite namespaces of one cluster.
pub(super) const FAVORITES_LIMIT: usize = 20;

/// A named set of clusters that the top bar selects with one click.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClusterGroup {
    pub name: String,
    /// The names of the contexts, in the sequence of the file.
    pub clusters: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_color_reads_six_hex_digits_and_writes_them_again() {
        assert_eq!(Rgb::parse("#3fb950"), Some(Rgb([0x3f, 0xb9, 0x50])));
        assert_eq!(Rgb::parse("#3FB950").map(|rgb| rgb.to_string()), Some("#3fb950".to_owned()));
        for bad in ["3fb950", "#3fb95", "#3fb9500", "#gggggg", "#3fb9é0", ""] {
            assert_eq!(Rgb::parse(bad), None, "{bad}");
        }
        assert_eq!(ConfirmRule::named("type-name"), Some(ConfirmRule::TypeName));
        assert_eq!(ConfirmRule::named("Dialog"), None);
    }
}
