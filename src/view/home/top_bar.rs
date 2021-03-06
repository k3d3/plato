use framebuffer::{Framebuffer, UpdateMode};
use view::{View, Event, Hub, Bus, ViewId, Align};
use view::icon::Icon;
use view::clock::Clock;
use view::home::sort_label::SortLabel;
use metadata::SortMethod;
use color::WHITE;
use app::Context;
use font::Fonts;
use geom::{Rectangle};

#[derive(Debug)]
pub struct TopBar {
    rect: Rectangle,
    children: Vec<Box<View>>,
}

impl TopBar {
    pub fn new(rect: Rectangle, sort_method: SortMethod, fonts: &mut Fonts) -> TopBar {
        let mut children = Vec::new();
        let side = rect.height() as i32;
        let root_icon = Icon::new("search",
                                  rect![rect.min, rect.min+side],
                                  WHITE,
                                  Align::Center,
                                  Event::Toggle(ViewId::SearchBar));
        children.push(Box::new(root_icon) as Box<View>);
        let mut clock_rect = rect![rect.max - pt!(3*side, side),
                                   rect.max - pt!(2*side, 0)];
        let clock_label = Clock::new(&mut clock_rect, fonts);
        let sort_label = SortLabel::new(rect![pt!(rect.min.x + side,
                                                  rect.min.y),
                                              pt!(clock_rect.min.x,
                                                  rect.max.y)],
                                        sort_method.label());
        children.push(Box::new(sort_label) as Box<View>);
        children.push(Box::new(clock_label) as Box<View>);
        let frontlight_icon = Icon::new("frontlight",
                                        rect![rect.max - pt!(2*side, side),
                                              rect.max - pt!(side, 0)],
                                        WHITE,
                                        Align::Center,
                                        Event::Show(ViewId::FrontlightMenu));
        children.push(Box::new(frontlight_icon) as Box<View>);
        let menu_rect = rect![rect.max-side, rect.max];
        let menu_icon = Icon::new("menu",
                                  menu_rect,
                                  WHITE,
                                  Align::Center,
                                  Event::ToggleNear(ViewId::MainMenu, menu_rect));
        children.push(Box::new(menu_icon) as Box<View>);
        TopBar {
            rect,
            children,
        }
    }

    // TODO: only update if needed
    pub fn update_icon(&mut self, search_visible: bool, hub: &Hub) {
        {
            let root_icon = self.children[0].as_mut().downcast_mut::<Icon>().unwrap();
            root_icon.name = if search_visible {
                "home".to_string()
            } else {
                "search".to_string()
            };
        }
        hub.send(Event::Render(*self.children[0].rect(), UpdateMode::Gui)).unwrap();
    }

    pub fn update_sort_label(&mut self, sort_method: SortMethod, hub: &Hub) {
        let sort_label = self.children[1].as_mut().downcast_mut::<SortLabel>().unwrap();
        sort_label.update(sort_method.label(), hub);
    }
}

impl View for TopBar {
    fn handle_event(&mut self, _evt: &Event, _hub: &Hub, _bus: &mut Bus, _context: &mut Context) -> bool {
        false
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
