extern crate serde_json;

mod top_bar;
mod sort_label;
mod matches_label;
mod summary;
mod category;
mod shelf;
mod book;
mod search_bar;
mod bottom_bar;

use std::f32;
use std::sync::mpsc;
use std::collections::BTreeSet;
use chrono::Local;
use metadata::{Metadata, SortMethod, sort};
use framebuffer::{Framebuffer, UpdateMode};
use view::{View, Event, Hub, Bus, ViewId, EntryId, EntryKind, THICKNESS_MEDIUM};
use view::filler::Filler;
use self::top_bar::TopBar;
use self::summary::Summary;
use self::shelf::Shelf;
use self::search_bar::SearchBar;
use view::common::{shift, locate, locate_by_id, toggle_main_menu};
use view::keyboard::{Keyboard, DEFAULT_LAYOUT};
use view::go_to_page::GoToPage;
use view::menu::Menu;
use self::bottom_bar::BottomBar;
use device::{CURRENT_DEVICE, BAR_SIZES};
use symbolic_path::SymbolicPath;
use helpers::save_json;
use unit::scale_by_dpi;
use app::Context;
use color::BLACK;
use geom::{Rectangle, CycleDir, halves, small_half};
use font::Fonts;
use errors::*;

#[derive(Debug)]
pub struct Home {
    rect: Rectangle,
    children: Vec<Box<View>>,
    current_page: usize,
    pages_count: usize,
    focus: Option<ViewId>,
    query: String,
    summary_size: u8,
    sort_method: SortMethod,
    reverse_order: bool,
    visible_books: Metadata,
    visible_categories: BTreeSet<String>,
    selected_categories: BTreeSet<String>,
    negated_categories: BTreeSet<String>,
}

impl Home {
    pub fn new(rect: Rectangle, hub: &Hub, context: &mut Context) -> Result<Home> {
        let dpi = CURRENT_DEVICE.dpi;
        let fonts = &mut context.fonts;
        let metadata = &mut context.metadata;
        let thickness = scale_by_dpi(THICKNESS_MEDIUM, dpi) as i32;
        let (small_thickness, big_thickness) = halves(thickness);
        let (_, height) = CURRENT_DEVICE.dims;
        let &(small_height, big_height) = BAR_SIZES.get(&(height, dpi)).unwrap();

        let mut children = Vec::new();

        let sort_method = SortMethod::Opened;
        let reverse_order = sort_method.reverse_order();

        // Not necessary?
        sort(metadata, sort_method, reverse_order);

        let visible_books = metadata.clone();
        let visible_categories = metadata.iter()
                                 .flat_map(|info| info.categories.iter())
                                 .map(|categ| categ.first_component().to_string())
                                 .collect::<BTreeSet<String>>();

        let selected_categories = BTreeSet::default();
        let negated_categories = BTreeSet::default();


        let max_lines = ((height - 3 * small_height) / big_height) as usize;
        let summary_size = context.settings.summary_size.max(1).min(max_lines as u8);
        let max_lines = max_lines - summary_size as usize + 1;
        let count = visible_books.len();
        let pages_count = (visible_books.len() as f32 / max_lines as f32).ceil() as usize;
        let current_page = 0;


        let top_bar = TopBar::new(rect![rect.min.x, rect.min.y,
                                        rect.max.x, rect.min.y + small_height as i32 - small_thickness],
                                  sort_method,
                                  fonts);
        children.push(Box::new(top_bar) as Box<View>);

        let separator = Filler::new(rect![rect.min.x, rect.min.y + small_height as i32 - small_thickness,
                                          rect.max.x, rect.min.y + small_height as i32 + big_thickness],
                                    BLACK);
        children.push(Box::new(separator) as Box<View>);

        let summary_height = small_height as i32 - thickness +
                             (summary_size - 1) as i32 * big_height as i32;
        let s_min_y = rect.min.y + small_height as i32 + big_thickness;
        let s_max_y = s_min_y + summary_height;

        let mut summary = Summary::new(rect![rect.min.x, s_min_y,
                                             rect.max.x, s_max_y]);

        let (tx, _rx) = mpsc::channel();

        summary.update(&visible_categories, &selected_categories,
                       &negated_categories, false, &tx, fonts);

        children.push(Box::new(summary) as Box<View>);

        let separator = Filler::new(rect![rect.min.x, s_max_y,
                                          rect.max.x, s_max_y + thickness],
                                    BLACK);
        children.push(Box::new(separator) as Box<View>);

        let mut shelf = Shelf::new(rect![rect.min.x, s_max_y + thickness,
                                         rect.max.x, rect.max.y - small_height as i32 - small_thickness]);

        shelf.update(&visible_books[..max_lines], &tx);

        children.push(Box::new(shelf) as Box<View>);

        let separator = Filler::new(rect![rect.min.x, rect.max.y - small_height as i32 - small_thickness,
                                          rect.max.x, rect.max.y - small_height as i32 + big_thickness],
                                    BLACK);
        children.push(Box::new(separator) as Box<View>);

        let bottom_bar = BottomBar::new(rect![rect.min.x, rect.max.y - small_height as i32 + big_thickness,
                                              rect.max.x, rect.max.y],
                                        current_page,
                                        pages_count,
                                        count,
                                        false);
        children.push(Box::new(bottom_bar) as Box<View>);

        hub.send(Event::Render(rect, UpdateMode::Full)).unwrap();

        Ok(Home {
            rect,
            children,
            current_page,
            pages_count,
            focus: None,
            query: "".to_string(),
            summary_size,
            sort_method,
            reverse_order,
            visible_books: visible_books,
            visible_categories: visible_categories,
            selected_categories: selected_categories,
            negated_categories: negated_categories,
        })
    }

    fn refresh_visibles(&mut self, update: bool, reset_page: bool, hub: &Hub, context: &mut Context) {
        let fonts = &mut context.fonts;
        let metadata = &mut context.metadata;

        self.visible_books = metadata.iter().filter(|info| {
            // TODO: case insensitive, scoped queries, etc.
            // NOTE: self.query.is_empty() isn't necessary here, because x.find("") is Some(0)
            (self.query.is_empty() || info.title.find(&self.query).is_some() ||
                                      info.subtitle.find(&self.query).is_some() ||
                                      info.author.find(&self.query).is_some() ||
                                      info.categories.iter().any(|c| c.find(&self.query).is_some()) ||
                                      info.file.path.to_str().and_then(|s| s.find(&self.query)).is_some()) &&
                (self.selected_categories.is_subset(&info.categories) ||
                 self.selected_categories.iter()
                                         .all(|s| info.categories
                                                      .iter().any(|c| c == s || c.is_descendant_of(s)))) &&
                (self.negated_categories.is_empty() ||
                 (self.negated_categories.is_disjoint(&info.categories) &&
                  info.categories.iter().all(|c| c.ancestors().all(|a| !self.negated_categories.contains(a)))))
        }).cloned().collect();

        self.visible_categories = self.visible_books.iter()
                                      .flat_map(|info| info.categories.clone()).collect();

        self.visible_categories = self.visible_categories.iter().map(|c| {
            if let Some(p) = c.parent() {
                if self.selected_categories.contains(p) {
                    return c.clone();
                }
            }
            c.first_component().to_string()
        }).collect();

        for s in &self.selected_categories {
            self.visible_categories.insert(s.clone());
            for a in s.ancestors() {
                self.visible_categories.insert(a.to_string());
            }
        }

        for n in &self.negated_categories {
            self.visible_categories.insert(n.clone());
            for a in n.ancestors() {
                self.visible_categories.insert(a.to_string());
            }
        }

        let max_lines = {
            let shelf = self.child(4).downcast_ref::<Shelf>().unwrap();
            shelf.max_lines
        };
        self.pages_count = (self.visible_books.len() as f32 / max_lines as f32).ceil() as usize;

        if reset_page  {
            self.current_page = 0;
        } else if self.current_page >= self.pages_count {
            self.current_page = self.pages_count - 1;
        }

        if update {
            self.update_summary(false, hub, fonts);
            self.update_shelf(false, hub);
            self.update_bottom_bar(hub);
        }
    }

    fn toggle_select_category(&mut self, categ: &str) {
        if self.selected_categories.contains(categ) {
            self.selected_categories.remove(categ);
        } else {
            self.selected_categories = self.selected_categories.iter().filter_map(|s| {
                if s.is_descendant_of(categ) || categ.is_descendant_of(s) {
                    None
                } else {
                    Some(s.clone())
                }
            }).collect();
            self.negated_categories = self.negated_categories.iter().filter_map(|n| {
                if n == categ || categ.is_descendant_of(n) {
                    None
                } else {
                    Some(n.clone())
                }
            }).collect();
            self.selected_categories.insert(categ.to_string());
        }
    }

    fn toggle_negate_category(&mut self, categ: &str) {
        if self.negated_categories.contains(categ) {
            self.negated_categories.remove(categ);
        } else {
            self.negated_categories = self.negated_categories.iter().filter_map(|s| {
                if s.is_descendant_of(categ) || categ.is_descendant_of(s) {
                    None
                } else {
                    Some(s.clone())
                }
            }).collect();
            self.selected_categories = self.selected_categories.iter().filter_map(|s| {
                if s == categ || s.is_descendant_of(categ) {
                    None
                } else {
                    Some(s.clone())
                }
            }).collect();
            self.negated_categories.insert(categ.to_string());
        }
    }

    fn toggle_negate_category_children(&mut self, parent: &str) {
        let mut children = Vec::new();

        for c in &self.visible_categories {
            if c.is_child_of(parent) {
                children.push(c.to_string());
            }
        }

        while let Some(c) = children.pop() {
            self.toggle_negate_category(&c);
        }
    }

    fn go_to_page(&mut self, index: usize, hub: &Hub) {
        if index >= self.pages_count {
            return;
        }
        self.current_page = index;
        self.update_shelf(false, hub);
        self.update_bottom_bar(hub);
    }

    fn set_current_page(&mut self, dir: CycleDir, hub: &Hub) {
        match dir {
            CycleDir::Next if self.current_page < self.pages_count - 1 => {
                self.current_page += 1;
            },
            CycleDir::Previous if self.current_page > 0 => {
                self.current_page -= 1;
            },
            _ => return,
        }

        self.update_shelf(false, hub);
        self.update_bottom_bar(hub);
    }

    fn update_summary(&mut self, was_resized: bool, hub: &Hub, fonts: &mut Fonts) {
        let summary = self.children[2].as_mut().downcast_mut::<Summary>().unwrap();
        summary.update(&self.visible_categories, &self.selected_categories, &self.negated_categories,
                       was_resized, hub, fonts);
    }

    fn update_shelf(&mut self, was_resized: bool, hub: &Hub) {
        let dpi = CURRENT_DEVICE.dpi;
        let (_, height) = CURRENT_DEVICE.dims;
        let &(_, big_height) = BAR_SIZES.get(&(height, dpi)).unwrap();
        let thickness = scale_by_dpi(THICKNESS_MEDIUM, dpi) as i32;

        let shelf = self.children[4].as_mut().downcast_mut::<Shelf>().unwrap();
        let max_lines = ((shelf.rect.height() + thickness as u32) / big_height) as usize;

        // TODO: extract this into a function and call this when the shelf is resized to avoid the
        // temporal dependency between update_shelf and update_bottom_bar
        if was_resized {
            let page_position = if self.visible_books.is_empty() {
                0.0
            } else {
                self.current_page as f32 * (shelf.max_lines as f32 /
                                            self.visible_books.len() as f32)
            };

            let mut page_guess = page_position * self.visible_books.len() as f32 / max_lines as f32;
            let page_ceil = page_guess.ceil();

            if (page_ceil - page_guess) < f32::EPSILON {
                page_guess = page_ceil;
            }

            self.pages_count = (self.visible_books.len() as f32 / max_lines as f32).ceil() as usize;
            self.current_page = (page_guess as usize).min(self.pages_count - 1);
        }

        let index_lower = self.current_page * max_lines;
        let index_upper = (index_lower + max_lines).min(self.visible_books.len());

        shelf.update(&self.visible_books[index_lower..index_upper], hub);
    }

    fn update_top_bar(&mut self, search_visible: bool, hub: &Hub) {
        if let Some(index) = locate::<TopBar>(self) {
            let top_bar = self.children[index].as_mut().downcast_mut::<TopBar>().unwrap();
            top_bar.update_icon(search_visible, hub);
            top_bar.update_sort_label(self.sort_method, hub);
        }
    }

    fn update_bottom_bar(&mut self, hub: &Hub) {
        if let Some(index) = locate::<BottomBar>(self) {
            let bottom_bar = self.children[index].as_mut().downcast_mut::<BottomBar>().unwrap();
            let filter = !self.query.is_empty() ||
                         !self.selected_categories.is_empty() ||
                         !self.negated_categories.is_empty();
            bottom_bar.update_matches_label(self.visible_books.len(), filter, hub);
            bottom_bar.update_page_label(self.current_page, self.pages_count, hub);
            bottom_bar.update_icons(self.current_page, self.pages_count, hub);
        }
    }

    fn toggle_keyboard(&mut self, enable: bool, update: bool, hub: &Hub, fonts: &mut Fonts) {
        let dpi = CURRENT_DEVICE.dpi;
        let (_, height) = CURRENT_DEVICE.dims;
        let &(small_height, big_height) = BAR_SIZES.get(&(height, dpi)).unwrap();
        let thickness = scale_by_dpi(THICKNESS_MEDIUM, dpi) as i32;
        let (small_thickness, big_thickness) = halves(thickness);
        let mut should_update_summary = false;
        let mut has_search_bar = false;

        if let Some(index) = locate::<Keyboard>(self) {
            if enable {
                return;
            }

            let kb_rect = *self.child(index).rect();

            self.children.drain(index - 1 .. index + 1);

            let delta_y = kb_rect.height() as i32 + thickness;

            {
                let shelf = self.child_mut(4).downcast_mut::<Shelf>().unwrap();
                shelf.rect.max.y += delta_y;
            }

            if index > 6 {
                has_search_bar = true;
                {
                    let separator = self.child_mut(5).downcast_mut::<Filler>().unwrap();
                    separator.rect += pt!(0, delta_y);
                }
                {
                    shift(self.child_mut(6), &pt!(0, delta_y));
                }
            }
        } else {
            if !enable {
                return;
            }

            let index = locate::<BottomBar>(self).unwrap() - 1;
            let mut kb_rect = rect![self.rect.min.x,
                                    self.rect.max.y - (small_height + 3 * big_height) as i32 + big_thickness,
                                    self.rect.max.x,
                                    self.rect.max.y - small_height as i32 - small_thickness];

            // TODO: render keyboard and separator
            let keyboard = Keyboard::new(&mut kb_rect, DEFAULT_LAYOUT.clone());
            self.children.insert(index, Box::new(keyboard) as Box<View>);

            let separator = Filler::new(rect![self.rect.min.x, kb_rect.min.y - thickness,
                                              self.rect.max.x, kb_rect.min.y],
                                        BLACK);
            self.children.insert(index, Box::new(separator) as Box<View>);

            let delta_y = kb_rect.height() as i32 + thickness;
            self.resize_summary(-delta_y, false, hub, fonts);
            should_update_summary = true;

            {
                let shelf = self.child_mut(4).downcast_mut::<Shelf>().unwrap();
                shelf.rect.max.y -= delta_y;
            }

            if index > 5 {
                has_search_bar = true;
                {
                    let separator = self.child_mut(5).downcast_mut::<Filler>().unwrap();
                    separator.rect -= pt!(0, delta_y);
                }
                {
                    shift(self.child_mut(6), &pt!(0, -delta_y));
                }
            }
        }

        if update {
            if should_update_summary {
                self.update_summary(true, hub, fonts);
                hub.send(Event::Render(*self.child(3).rect(), UpdateMode::Gui)).unwrap();
            }
            self.update_shelf(true, hub);
            self.update_bottom_bar(hub);
            if enable {
                if has_search_bar {
                    for i in 5..9 {
                        hub.send(Event::Render(*self.child(i).rect(), UpdateMode::Gui)).unwrap();
                    }
                } else {
                    for i in 5..7 {
                        hub.send(Event::Render(*self.child(i).rect(), UpdateMode::Gui)).unwrap();
                    }
                }
            } else {
                if has_search_bar {
                    for i in 5..7 {
                        hub.send(Event::Render(*self.child(i).rect(), UpdateMode::Gui)).unwrap();
                    }
                }
            }
        }
    }

    fn toggle_search_bar(&mut self, enable: Option<bool>, update: bool, hub: &Hub, fonts: &mut Fonts) {
        let dpi = CURRENT_DEVICE.dpi;
        let (_, height) = CURRENT_DEVICE.dims;
        let &(small_height, big_height) = BAR_SIZES.get(&(height, dpi)).unwrap();
        let thickness = scale_by_dpi(THICKNESS_MEDIUM, dpi) as i32;
        let small_thickness = small_half(thickness);

        let delta_y = (big_height - small_height) as i32;
        let search_visible: bool;

        if let Some(index) = locate::<SearchBar>(self) {
            if let Some(true) = enable {
                return;
            }

            if let Some(ViewId::SearchInput) = self.focus {
                self.focus = None;
                self.toggle_keyboard(false, false, hub, fonts);
            }

            self.children.drain(index - 1 .. index + 1);

            {
                let shelf = self.child_mut(4).downcast_mut::<Shelf>().unwrap();
                shelf.rect.max.y += small_height as i32;
            }

            self.resize_summary(-delta_y, false, hub, fonts);
            self.query.clear();

            search_visible = false;
        } else {
            if let Some(false) = enable {
                return;
            }

            let sp_rect = *self.child(5).rect() - pt!(0, small_height as i32);

            let search_bar = SearchBar::new(rect![self.rect.min.x, sp_rect.max.y,
                                                  self.rect.max.x,
                                                  sp_rect.max.y + small_height as i32 - small_thickness]);

            self.children.insert(5, Box::new(search_bar) as Box<View>);

            let separator = Filler::new(sp_rect, BLACK);
            self.children.insert(5, Box::new(separator) as Box<View>);

            // move the shelf's bottom edge
            {
                let shelf = self.child_mut(4).downcast_mut::<Shelf>().unwrap();
                shelf.rect.max.y -= small_height as i32;
            }

            if locate::<Keyboard>(self).is_none() {
                self.toggle_keyboard(true, false, hub, fonts);
            }

            self.focus = Some(ViewId::SearchInput);
            hub.send(Event::Focus(Some(ViewId::SearchInput))).unwrap();

            self.resize_summary(delta_y - big_height as i32, false, hub, fonts);
            search_visible = true;
        }

        if update {
            if search_visible {
                // TODO: don't update if the keyboard is already present
                for i in [3usize, 5, 6, 7, 8].iter().cloned() {
                    hub.send(Event::Render(*self.child(i).rect(), UpdateMode::Gui)).unwrap();
                }
            } else {
                for i in [3usize, 5].iter().cloned() {
                    hub.send(Event::Render(*self.child(i).rect(), UpdateMode::Gui)).unwrap();
                }
            }

            self.update_top_bar(search_visible, hub);
            self.update_summary(true, hub, fonts);
            self.update_shelf(true, hub);
            self.update_bottom_bar(hub);
        }
    }

    fn toggle_go_to_page(&mut self, enable: Option<bool>, hub: &Hub, fonts: &mut Fonts) {
        if let Some(index) = locate_by_id(self, ViewId::GoToPage) {
            if let Some(true) = enable {
                return;
            }
            hub.send(Event::Expose(*self.child(index).rect())).unwrap();
            self.children.remove(index);
            if let Some(ViewId::GoToPageInput) = self.focus {
                self.focus = None;
                self.toggle_keyboard(false, true, hub, fonts);
            }
        } else {
            if let Some(false) = enable {
                return;
            }
            let anchor = self.child(4).child(1).rect().center();
            let go_to_page = GoToPage::new(&anchor, self.pages_count, fonts);
            hub.send(Event::Render(*go_to_page.rect(), UpdateMode::Gui)).unwrap();
            hub.send(Event::Focus(Some(ViewId::GoToPageInput))).unwrap();
            self.focus = Some(ViewId::GoToPageInput);
            self.children.push(Box::new(go_to_page) as Box<View>);
        }
    }

    fn toggle_sort_menu(&mut self, rect: Rectangle, enable: Option<bool>, hub: &Hub, fonts: &mut Fonts) {
        if let Some(index) = locate_by_id(self, ViewId::SortMenu) {
            if let Some(true) = enable {
                return;
            }
            hub.send(Event::Expose(*self.child(index).rect())).unwrap();
            self.children.remove(index);
        } else {
            if let Some(false) = enable {
                return;
            }
            let entries = &[EntryKind::RadioButton("Date Opened".to_string(),
                                                   EntryId::Sort(SortMethod::Opened),
                                                   self.sort_method == SortMethod::Opened),
                            EntryKind::RadioButton("Date Added".to_string(),
                                                   EntryId::Sort(SortMethod::Added),
                                                   self.sort_method == SortMethod::Added),
                            EntryKind::RadioButton("Progress".to_string(),
                                                   EntryId::Sort(SortMethod::Progress),
                                                   self.sort_method == SortMethod::Progress),
                            EntryKind::RadioButton("Author".to_string(),
                                                   EntryId::Sort(SortMethod::Author),
                                                   self.sort_method == SortMethod::Author),
                            EntryKind::RadioButton("File Size".to_string(),
                                                   EntryId::Sort(SortMethod::Size),
                                                   self.sort_method == SortMethod::Size),
                            EntryKind::RadioButton("File Type".to_string(),
                                                   EntryId::Sort(SortMethod::Kind),
                                                   self.sort_method == SortMethod::Kind),
                            EntryKind::Separator,
                            EntryKind::CheckBox("Reverse Order".to_string(),
                                                EntryId::ReverseOrder, self.reverse_order)];
            let sort_menu = Menu::new(rect, ViewId::SortMenu, entries, fonts);
            hub.send(Event::Render(*sort_menu.rect(), UpdateMode::Gui)).unwrap();
            self.children.push(Box::new(sort_menu) as Box<View>);
        }
    }

    fn toggle_matches_menu(&mut self, rect: Rectangle, enable: Option<bool>, hub: &Hub, fonts: &mut Fonts) {
        if let Some(index) = locate_by_id(self, ViewId::MatchesMenu) {
            if let Some(true) = enable {
                return;
            }
            hub.send(Event::Expose(*self.child(index).rect())).unwrap();
            self.children.remove(index);
        } else {
            if let Some(false) = enable {
                return;
            }
            let entries = &[EntryKind::Command("Export".to_string(),
                                               EntryId::ExportMatches)];
            let matches_menu = Menu::new(rect, ViewId::MatchesMenu, entries, fonts);
            hub.send(Event::Render(*matches_menu.rect(), UpdateMode::Gui)).unwrap();
            self.children.push(Box::new(matches_menu) as Box<View>);
        }
    }

    // Relatively moves the bottom edge of the summary
    // And consequently moves the top edge of the shelf
    // and the separator between them.
    fn resize_summary(&mut self, delta_y: i32, update: bool, hub: &Hub, fonts: &mut Fonts) {
        let dpi = CURRENT_DEVICE.dpi;
        let (_, height) = CURRENT_DEVICE.dims;
        let &(small_height, big_height) = BAR_SIZES.get(&(height, dpi)).unwrap();
        let thickness = scale_by_dpi(THICKNESS_MEDIUM, dpi) as i32;

        let min_height = if locate::<SearchBar>(self).is_some() {
            big_height as i32 - thickness
        } else {
            small_height as i32 - thickness
        };

        let (current_height, next_height) = {
            let summary = self.child(2).downcast_ref::<Summary>().unwrap();
            let shelf = self.child(4).downcast_ref::<Shelf>().unwrap();
            let max_height = shelf.rect.max.y - summary.rect.min.y - big_height as i32;
            let current_height = summary.rect.height() as i32;
            let size_factor = ((current_height + delta_y - min_height) as f32 / big_height as f32).round() as i32;
            let next_height = max_height.min(min_height.max(min_height + size_factor * big_height as i32));
            (current_height, next_height)
        };

        if current_height == next_height {
            return;
        }

        // move the summary's bottom edge
        let delta_y = {
            let summary = self.child_mut(2).downcast_mut::<Summary>().unwrap();
            let last_max_y = summary.rect.max.y;
            summary.rect.max.y = summary.rect.min.y + next_height;
            summary.rect.max.y - last_max_y
        };

        // move the separator
        {
            let separator = self.child_mut(3).downcast_mut::<Filler>().unwrap();
            separator.rect += pt!(0, delta_y);
        }

        // move the shelf's top edge
        {
            let shelf = self.child_mut(4).downcast_mut::<Shelf>().unwrap();

            shelf.rect.min.y += delta_y;
        }

        if update {
            hub.send(Event::Render(*self.child(3).rect(), UpdateMode::Gui)).unwrap();
            self.update_summary(true, hub, fonts);
            self.update_shelf(true, hub);
            self.update_bottom_bar(hub);
        }
    }

    fn set_reverse_order(&mut self, value: bool, hub: &Hub, context: &mut Context) {
        self.reverse_order = value;
        self.sort(true, &mut context.metadata, hub);
    }

    fn set_sort_method(&mut self, sort_method: SortMethod, hub: &Hub, context: &mut Context) {
        self.sort_method = sort_method;
        self.reverse_order = sort_method.reverse_order();
        self.sort(true, &mut context.metadata, hub);
    }

    fn sort(&mut self, reset_page: bool, metadata: &mut Metadata, hub: &Hub) {
        if reset_page {
            self.current_page = 0;
        }
        sort(metadata, self.sort_method, self.reverse_order);
        sort(&mut self.visible_books, self.sort_method, self.reverse_order);
        self.update_shelf(false, hub);
        let search_visible = locate::<SearchBar>(self).is_some();
        self.update_top_bar(search_visible, hub);
        self.update_bottom_bar(hub);
    }

    fn reseed(&mut self, hub: &Hub, context: &mut Context) {
        self.refresh_visibles(false, false, hub, context);
        self.sort(false, &mut context.metadata, hub);
        hub.send(Event::ClockTick).unwrap();
        hub.send(Event::Render(self.rect, UpdateMode::Gui)).unwrap();
    }

    fn export_matches(&self, context: &mut Context) {
        let path = context.settings
                          .library_path
                          .join(&Local::now()
                                .format(".metadata-matches_%Y%m%d_%H%M%S.json").to_string());
        save_json(&self.visible_books, path).map_err(|e| {
            eprintln!("Couldn't export matches: {}.", e);
        }).ok();
    }
}

// TODO: make the update_* and resize_* methods take a mutable bit fields as argument and make a
// generic method for updating everything based on the bit field to avoid needlessly updating
// things multiple times

impl View for Home {
    fn handle_event(&mut self, evt: &Event, hub: &Hub, _bus: &mut Bus, context: &mut Context) -> bool {
        match *evt {
            Event::Focus(v) => {
                self.focus = v;
                self.toggle_keyboard(true, true, hub, &mut context.fonts);
                false // let the event reach every input view
            },
            // TODO: handle other views
            Event::Show(ViewId::Keyboard) => {
                self.toggle_keyboard(true, true, hub, &mut context.fonts);
                true
            },
            Event::Toggle(ViewId::GoToPage) => {
                self.toggle_go_to_page(None, hub, &mut context.fonts);
                true
            },
            Event::Toggle(ViewId::SearchBar) => {
                self.toggle_search_bar(None, true, hub, &mut context.fonts);
                true
            },
            Event::ToggleNear(ViewId::SortMenu, rect) => {
                self.toggle_sort_menu(rect, None, hub, &mut context.fonts);
                true
            },
            Event::ToggleNear(ViewId::MainMenu, rect) => {
                toggle_main_menu(self, rect, None, hub, context);
                true
            },
            Event::ToggleNear(ViewId::MatchesMenu, rect) => {
                self.toggle_matches_menu(rect, None, hub, &mut context.fonts);
                true
            },
            Event::Close(ViewId::SearchBar) => {
                self.toggle_search_bar(Some(false), true, hub, &mut context.fonts);
                self.refresh_visibles(true, true, hub, context);
                true
            },
            Event::Close(ViewId::SortMenu) => {
                self.toggle_sort_menu(Rectangle::default(), Some(false), hub, &mut context.fonts);
                true
            },
            Event::Close(ViewId::MatchesMenu) => {
                self.toggle_matches_menu(Rectangle::default(), Some(false), hub, &mut context.fonts);
                true
            },
            Event::Close(ViewId::MainMenu) => {
                toggle_main_menu(self, Rectangle::default(), Some(false), hub, context);
                true
            },
            Event::Close(ViewId::GoToPage) => {
                self.toggle_go_to_page(Some(false), hub, &mut context.fonts);
                true
            },
            Event::Select(EntryId::Sort(sort_method)) => {
                self.set_sort_method(sort_method, hub, context);
                true
            },
            Event::Select(EntryId::ReverseOrder) => {
                let next_value = !self.reverse_order;
                self.set_reverse_order(next_value, hub, context);
                true
            },
            Event::Select(EntryId::ExportMatches) => {
                self.export_matches(context);
                true
            },
            Event::Submit(ViewId::SearchInput, ref query) => {
                self.query = query.clone();
                // TODO: avoid updating things twice
                self.toggle_keyboard(false, true, hub, &mut context.fonts);
                self.refresh_visibles(true, true, hub, context);
                true
            },
            Event::ResizeSummary(delta_y) => {
                self.resize_summary(delta_y, true, hub, &mut context.fonts);
                true
            },
            Event::ToggleSelectCategory(ref categ) => {
                self.toggle_select_category(categ);
                self.refresh_visibles(true, true, hub, context);
                true
            },
            Event::ToggleNegateCategory(ref categ) => {
                self.toggle_negate_category(categ);
                self.refresh_visibles(true, true, hub, context);
                true
            },
            Event::ToggleNegateCategoryChildren(ref categ) => {
                self.toggle_negate_category_children(categ);
                self.refresh_visibles(true, true, hub, context);
                true
            },
            Event::GoTo(index) => {
                self.go_to_page(index, hub);
                true
            },
            Event::Page(dir) => {
                self.set_current_page(dir, hub);
                true
            },
            Event::Back => {
                self.reseed(hub, context);
                true
            },
            _ => false,
        }
    }

    fn render(&self, _fb: &mut Framebuffer, _fonts: &mut Fonts) {
    }

    fn rect(&self) -> &Rectangle {
        &self.rect
    }

    fn rect_mut(&mut self) -> &mut Rectangle {
        &mut self.rect
    }

    fn children(&self) -> &Vec<Box<View>> {
        &self.children
    }

    fn children_mut(&mut self) -> &mut Vec<Box<View>> {
        &mut self.children
    }
}
