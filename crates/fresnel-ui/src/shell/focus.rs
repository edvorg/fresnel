//! The focus of the keys between the areas of the window (plan section 11.5): the sidebar,
//! the table, the panel and the dock. A command puts the focus into an area; then the
//! arrows of egui move it between the entries of the area, and Space or Enter acts. The
//! table has its own keys for its rows.

use fresnel_core::commands::Area;

/// The area of the last focus command, and the area that takes the focus in this frame.
#[derive(Debug)]
pub(super) struct Focus {
    area: Area,
    wanted: Option<Area>,
}

impl Default for Focus {
    fn default() -> Self {
        Self { area: Area::Table, wanted: None }
    }
}

impl Focus {
    /// Asks for the focus of an area in this frame.
    pub(super) fn to(&mut self, area: Area) {
        (self.area, self.wanted) = (area, Some(area));
    }

    /// The area after the last one, or the one before it.
    pub(super) fn step(&mut self, forward: bool) -> Area {
        let count = Area::ALL.len();
        let now = Area::ALL.iter().position(|area| *area == self.area).unwrap_or_default();
        let next = if forward { (now + 1) % count } else { (now + count - 1) % count };
        let area = Area::ALL.get(next).copied().unwrap_or(Area::Table);
        self.to(area);
        area
    }

    /// True once when the area takes the focus in this frame.
    pub(super) fn take(&mut self, area: Area) -> bool {
        let wanted = self.wanted == Some(area);
        if wanted {
            self.wanted = None;
        }
        wanted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_next_area_goes_round_and_an_area_takes_its_focus_one_time() {
        let mut focus = Focus::default();
        assert_eq!((focus.step(true), focus.step(true), focus.step(true)), (Area::Panel, Area::Dock, Area::Sidebar));
        assert_eq!(focus.step(false), Area::Dock);
        assert!(!focus.take(Area::Sidebar) && focus.take(Area::Dock) && !focus.take(Area::Dock));
    }
}
