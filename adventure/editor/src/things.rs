//! Placing things on the maps (characters, items, doors) and setting up their rules.

use adventure_format::{
    Action, Animation, Condition, Dir, Id, Motion, Place, Pos, Rule, SoundRef, SpriteRef, Thing,
};
use eframe::egui::{
    self, load::SizedTexture, vec2, Align, Color32, ComboBox, Frame, Layout, Margin, Rect,
    RichText, Sense, TextureHandle, Ui,
};

use crate::project::{FlagDef, ItemDef, Project, ThingDef};
use crate::theme::*;
use crate::{ensure_sprites, field, header, Editor, Pick, Tool};

/// What clicking an empty cell places.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stamp {
    Sprite(SpriteRef),
    /// An invisible thing, e.g. the spot in front of a door
    Spot,
}

/// Which rules of a thing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    Bump,
    Step,
}

/// Names of what rules can refer to, for the lists to choose from.
struct Lists {
    sounds: Vec<String>,
    sprites: Vec<String>,
    items: Vec<(Id, String)>,
    flags: Vec<(Id, String)>,
    things: Vec<(Id, String)>,
    maps: Vec<(Id, String)>,
    /// The thing being edited
    this: Id,
    /// The ID a new item or flag would get
    next: Id,
}

/// Something the rules editor needs done to the project after editing.
#[derive(Default)]
struct Requests {
    new_item: bool,
    new_flag: bool,
    /// Pick a door's target on the map: (trigger, rule, action)
    pick: Option<(Trigger, usize, usize)>,
}

impl Lists {
    fn new(project: &Project, this: Id) -> Lists {
        let maps: Vec<(Id, String)> = project
            .maps
            .iter()
            .map(|m| (m.id, m.name.clone()))
            .collect();
        Lists {
            sounds: project.sounds.iter().map(|s| s.name.clone()).collect(),
            sprites: project.sprites.iter().map(|s| s.name.clone()).collect(),
            items: project
                .items
                .iter()
                .map(|i| (i.id, i.name.clone()))
                .collect(),
            flags: project
                .flags
                .iter()
                .map(|f| (f.id, f.name.clone()))
                .collect(),
            things: project
                .maps
                .iter()
                .flat_map(|map| {
                    map.things
                        .iter()
                        .map(move |t| (t.thing.id, format!("{} · {}", t.name, map.name)))
                })
                .collect(),
            maps,
            this,
            next: Id(project.next_id + 1),
        }
    }

    fn name(list: &[(Id, String)], id: Id) -> String {
        list.iter()
            .find(|(i, _)| *i == id)
            .map_or_else(|| "?".into(), |(_, n)| n.clone())
    }
}

impl Editor {
    /// The Things tab: what to place, and what's on this map.
    pub(crate) fn things_palette(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let Some(open) = &mut self.open else { return };
        ensure_sprites(open, &ctx);
        ui.label(muted("Click a picture, then a cell on the map.").small());
        ui.add_space(4.0);
        let gap = 8.0;
        let side = ((ui.available_width() - 3.0 * gap) / 4.0).floor();
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing = vec2(gap, gap);
            ui.horizontal_wrapped(|ui| {
                let stamps = (0..open.project.sprites.len())
                    .map(|i| Stamp::Sprite(SpriteRef(i as u16)))
                    .chain([Stamp::Spot]);
                for stamp in stamps {
                    let (rect, response) = ui.allocate_exact_size(vec2(side, side), Sense::click());
                    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
                    ui.painter().rect_filled(rect, 10, PANEL2);
                    let name = match stamp {
                        Stamp::Sprite(sprite) => {
                            if let Some(Some((texture, uv))) =
                                open.sprite_pictures.get(sprite.0 as usize)
                            {
                                picture(ui, texture, *uv, rect.shrink(4.0));
                            }
                            open.project.sprites[sprite.0 as usize].name.clone()
                        }
                        Stamp::Spot => {
                            spot(ui, rect.shrink(6.0));
                            "An invisible spot, e.g. in front of a door".into()
                        }
                    };
                    if stamp == self.stamp {
                        ring(ui, rect, 12);
                    } else if response.hovered() {
                        ui.painter().rect_stroke(
                            rect,
                            10,
                            egui::Stroke::new(1.5, MUTED),
                            egui::StrokeKind::Outside,
                        );
                    }
                    if response.on_hover_text(name).clicked() {
                        self.stamp = stamp;
                        // Choosing what to place means placing next
                        self.tool = Tool::Place;
                    }
                }
            });
        });
        ui.add_space(12.0);
        heading(ui, "On this map");
        ui.add_space(4.0);
        let map = &open.project.maps[self.map];
        if map.things.is_empty() {
            ui.label(muted("Nothing placed yet.").small());
        }
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                for (i, thing) in map.things.iter().enumerate() {
                    let on = self.thing == Some(i);
                    let look = Frame::new()
                        .fill(if on { SELECT } else { Color32::TRANSPARENT })
                        .corner_radius(10)
                        .inner_margin(Margin::symmetric(6, 4));
                    let row = surface(ui, look, (!on).then_some(HOVER), |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            let (rect, _) =
                                ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
                            match thing.thing.look {
                                Some(sprite) => {
                                    if let Some(Some((texture, uv))) =
                                        open.sprite_pictures.get(sprite.0 as usize)
                                    {
                                        picture(ui, texture, *uv, rect);
                                    }
                                }
                                None => spot(ui, rect),
                            }
                            ui.label(RichText::new(&thing.name).color(if on {
                                TEXT
                            } else {
                                MUTED
                            }));
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                let at = thing.thing.at;
                                ui.label(muted(format!("{}, {}", at.x, at.y)).small());
                            });
                        });
                    });
                    if row.clicked() {
                        self.thing = Some(i);
                    }
                }
            });
    }

    /// A click on the map while placing things.
    pub(crate) fn things_click(&mut self, at: Pos, input: &crate::CanvasInput) {
        let Some(open) = &mut self.open else { return };
        let map_index = self.map;
        let hit = open.project.maps[map_index]
            .things
            .iter()
            .position(|t| t.thing.at == at);

        if let Some(pick) = self.picking {
            if input.clicked {
                // Where the player arrives, facing the way chosen in the rule
                let map = open.project.maps[map_index].id;
                if let Some(Action::Teleport(target)) = action_mut(&mut open.project, pick) {
                    target.map = map;
                    target.at = at;
                }
                self.map = pick.map;
                self.thing = Some(pick.thing);
                self.picking = None;
                self.fit = true;
            }
            return;
        }

        let tool = self.tool;
        if tool == Tool::Start {
            if input.clicked {
                let map = open.project.maps[map_index].id;
                open.project.start = Place {
                    map,
                    at,
                    facing: Dir::Down,
                };
            }
            return;
        }
        if tool == Tool::Remove {
            if let (true, Some(i)) = (input.clicked, hit) {
                self.delete_thing(map_index, i);
            }
            return;
        }

        // Select and Place: dragging an object moves it
        if input.drag_started {
            self.dragging = hit;
            if hit.is_some() {
                self.thing = hit;
            }
        }
        if input.drag_stopped {
            if let Some(i) = self.dragging.take() {
                let objects = &mut open.project.maps[map_index].things;
                if objects.iter().all(|t| t.thing.at != at) {
                    objects[i].thing.at = at;
                }
            }
            return;
        }
        if !input.clicked {
            return;
        }
        match (hit, tool) {
            (Some(i), _) => self.thing = Some(i),
            (None, Tool::Place) => {
                if let Stamp::Sprite(sprite) = self.stamp {
                    if sprite.0 as usize >= open.project.sprites.len() {
                        self.fail("Add a picture first: World settings › Pictures");
                        return;
                    }
                }
                let id = open.project.new_id();
                let (look, name, solid) = match self.stamp {
                    Stamp::Sprite(sprite) => (
                        Some(sprite),
                        open.project
                            .sprites
                            .get(sprite.0 as usize)
                            .map_or("Object".into(), |s| s.name.clone()),
                        true,
                    ),
                    Stamp::Spot => (None, "Spot".to_string(), false),
                };
                let objects = &mut open.project.maps[map_index].things;
                objects.push(ThingDef {
                    name,
                    thing: Thing {
                        id,
                        at,
                        look,
                        solid,
                        motion: Motion::Still,
                        shown_if: Condition::Always,
                        on_bump: Vec::new(),
                        on_step: Vec::new(),
                    },
                });
                self.thing = Some(objects.len() - 1);
            }
            (None, _) => self.thing = None,
        }
    }

    /// Removes a thing, and the rules' "make disappear" of it everywhere.
    pub(crate) fn delete_thing(&mut self, map: usize, index: usize) {
        let Some(open) = &mut self.open else { return };
        let Some(gone) = open
            .project
            .maps
            .get_mut(map)
            .map(|m| m.things.remove(index))
        else {
            return;
        };
        for map in &mut open.project.maps {
            for thing in &mut map.things {
                for rule in thing
                    .thing
                    .on_bump
                    .iter_mut()
                    .chain(&mut thing.thing.on_step)
                {
                    rule.then
                        .retain(|action| *action != Action::Hide(gone.thing.id));
                }
            }
        }
        self.thing = None;
        self.say(format!("Removed {}", gone.name));
    }

    /// The right panel while placing things: the selected one, with its rules.
    pub(crate) fn thing_inspector(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let mut delete = None;
        let mut requests = Requests::default();
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            let Some(open) = &mut self.open else { return };
            ensure_sprites(open, &ctx);
            let map_index = self.map;
            let Some(index) = self.thing.filter(|&i| i < open.project.maps[map_index].things.len()) else {
                header(ui, "Object", "nothing selected");
                ui.label(muted("Click an object on the map to see it here, or choose a picture on the left and click an empty cell to place one.").small());
                return;
            };
            let original = open.project.maps[map_index].things[index].clone();
            let mut def = original.clone();
            let lists = Lists::new(&open.project, def.thing.id);
            let at = def.thing.at;
            header(ui, "Object", &format!("this one, at {}, {}", at.x, at.y));

            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(vec2(64.0, 64.0), Sense::hover());
                ui.painter().rect_filled(rect, 12, PANEL2);
                match def.thing.look {
                    Some(sprite) => {
                        if let Some(Some((texture, uv))) = open.sprite_pictures.get(sprite.0 as usize) {
                            picture(ui, texture, *uv, rect.shrink(6.0));
                        }
                    }
                    None => spot(ui, rect.shrink(10.0)),
                }
                ui.vertical(|ui| {
                    ui.label(muted("Name").small());
                    text_input(ui, &mut def.name, "what you call it");
                });
            });
            field(ui, "Picture", |ui| {
                let current = def
                    .thing
                    .look
                    .and_then(|s| lists.sprites.get(s.0 as usize).cloned())
                    .unwrap_or_else(|| "Invisible".into());
                ComboBox::from_id_salt("look").selected_text(current).width(ui.available_width()).show_ui(ui, |ui| {
                    ui.selectable_value(&mut def.thing.look, None, "Invisible");
                    for (i, name) in lists.sprites.iter().enumerate() {
                        ui.selectable_value(&mut def.thing.look, Some(SpriteRef(i as u16)), name);
                    }
                });
            });
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.label(strong("Solid"));
                    let what = if def.thing.solid {
                        "the player bumps into it, like a character"
                    } else {
                        "the player walks onto it, like an item or a door"
                    };
                    ui.label(muted(what).small());
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    switch(ui, &mut def.thing.solid);
                });
            });
            field(ui, "Moves", |ui| motion_ui(ui, &mut def.thing.motion));
            field(ui, "Is there", |ui| {
                let mut only_if = def.thing.shown_if != Condition::Always;
                ui.horizontal(|ui| {
                    if ui.selectable_label(!only_if, "Always").clicked() {
                        only_if = false;
                    }
                    if ui.selectable_label(only_if, "Only if…").clicked() && !only_if {
                        only_if = true;
                    }
                });
                if !only_if {
                    def.thing.shown_if = Condition::Always;
                } else {
                    if def.thing.shown_if == Condition::Always {
                        def.thing.shown_if = default_condition(&lists, &mut requests);
                    }
                    condition_ui(ui, "shown", &mut def.thing.shown_if, &lists, &mut requests, false);
                }
            });

            ui.add_space(6.0);
            rules_ui(ui, Trigger::Bump, &mut def.thing.on_bump, &lists, &mut requests);
            ui.add_space(6.0);
            rules_ui(ui, Trigger::Step, &mut def.thing.on_step, &lists, &mut requests);
            ui.add_space(8.0);
            if ui.add(danger(format!("{TRASH}  Delete object"))).clicked() {
                delete = Some(index);
            }

            if requests.new_item {
                let id = open.project.new_id();
                let icon = def.thing.look.unwrap_or(SpriteRef(0));
                let name = format!("Item {}", open.project.items.len() + 1);
                open.project.items.push(ItemDef { id, name, icon });
            }
            if requests.new_flag {
                let id = open.project.new_id();
                let name = format!("Flag {}", open.project.flags.len() + 1);
                open.project.flags.push(FlagDef { id, name });
            }
            if def != original {
                open.project.maps[map_index].things[index] = def;
            }
            if let Some((trigger, rule, action)) = requests.pick {
                self.picking = Some(Pick { map: map_index, thing: index, trigger, rule, action });
            }
        });
        if let Some(index) = delete {
            self.delete_thing(self.map, index);
        }
    }
}

/// The action a door's target is being picked for.
fn action_mut(project: &mut Project, pick: Pick) -> Option<&mut Action> {
    let thing = &mut project
        .maps
        .get_mut(pick.map)?
        .things
        .get_mut(pick.thing)?
        .thing;
    let rules = match pick.trigger {
        Trigger::Bump => &mut thing.on_bump,
        Trigger::Step => &mut thing.on_step,
    };
    rules.get_mut(pick.rule)?.then.get_mut(pick.action)
}

pub fn picture(ui: &Ui, texture: &TextureHandle, uv: Rect, rect: Rect) {
    egui::Image::from_texture(SizedTexture::new(texture.id(), rect.size()))
        .uv(uv)
        .paint_at(ui, rect);
}

/// How an invisible thing is shown in the editor: a dashed square with a door.
pub fn spot(ui: &Ui, rect: Rect) {
    let painter = ui.painter();
    let stroke = egui::Stroke::new(1.5, MUTED);
    let corners = [
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
        rect.left_top(),
    ];
    for pair in corners.windows(2) {
        painter.extend(egui::Shape::dashed_line(
            &[pair[0], pair[1]],
            stroke,
            4.0,
            3.0,
        ));
    }
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        DOOR,
        egui::FontId::proportional(rect.height() * 0.55),
        MUTED,
    );
}

fn motion_ui(ui: &mut Ui, motion: &mut Motion) {
    let kind = match motion {
        Motion::Still => 0,
        Motion::Wander { .. } => 1,
        Motion::Path(_) => 2,
    };
    ui.horizontal(|ui| {
        for (k, label) in [(0, "Stays"), (1, "Wanders"), (2, "Walks a path")] {
            if ui.selectable_label(kind == k, label).clicked() && kind != k {
                *motion = match k {
                    0 => Motion::Still,
                    1 => Motion::Wander { radius: 2 },
                    _ => Motion::Path(vec![Dir::Right, Dir::Left]),
                };
            }
        }
    });
    match motion {
        Motion::Still => {}
        Motion::Wander { radius } => {
            ui.horizontal(|ui| {
                ui.label(muted("up to"));
                ui.add(egui::DragValue::new(radius).range(1..=10));
                ui.label(muted("tiles from where it's placed"));
            });
        }
        Motion::Path(steps) => {
            ui.horizontal_wrapped(|ui| {
                let arrows: String = steps
                    .iter()
                    .map(|&d| arrow(d))
                    .collect::<Vec<_>>()
                    .join(" ");
                ui.label(RichText::new(arrows).size(16.0));
            });
            ui.horizontal(|ui| {
                ui.label(muted("add a step"));
                for dir in [Dir::Up, Dir::Down, Dir::Left, Dir::Right] {
                    if ui
                        .add(quiet(RichText::new(arrow(dir)).size(16.0)))
                        .clicked()
                    {
                        steps.push(dir);
                    }
                }
                if steps.len() > 1 && ui.add(quiet(muted("undo step"))).clicked() {
                    steps.pop();
                }
            });
        }
    }
}

fn arrow(dir: Dir) -> &'static str {
    match dir {
        Dir::Up => ARROW_UP,
        Dir::Down => ARROW_DOWN,
        Dir::Left => ARROW_LEFT,
        Dir::Right => ARROW_RIGHT,
    }
}

fn rules_ui(
    ui: &mut Ui,
    trigger: Trigger,
    rules: &mut Vec<Rule>,
    lists: &Lists,
    requests: &mut Requests,
) {
    let (title, hint) = match trigger {
        Trigger::Bump => (
            "When the player bumps into it",
            "or presses A in front of it",
        ),
        Trigger::Step => (
            "When the player steps onto it",
            "for objects that aren't solid",
        ),
    };
    ui.label(strong(title));
    ui.label(muted(hint).small());
    let mut remove = None;
    let mut raise = None;
    for (r, rule) in rules.iter_mut().enumerate() {
        Frame::new()
            .fill(PANEL2)
            .corner_radius(12)
            .inner_margin(Margin::same(10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let unconditional = rule.when == Condition::Always;
                    let label = match (r, unconditional) {
                        (0, true) => "Every time",
                        (_, true) => "Otherwise",
                        (0, false) => "If",
                        (_, false) => "Else if",
                    };
                    ui.label(strong(label));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add(quiet(CROSS))
                            .on_hover_text("Remove this rule")
                            .clicked()
                        {
                            remove = Some(r);
                        }
                        if r > 0
                            && ui
                                .add(quiet(ARROW_UP))
                                .on_hover_text("Check this rule earlier")
                                .clicked()
                        {
                            raise = Some(r);
                        }
                    });
                });
                let salt = format!("{trigger:?} {r}");
                condition_ui(ui, &salt, &mut rule.when, lists, requests, true);
                let mut drop = None;
                for (a, action) in rule.then.iter_mut().enumerate() {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(muted(ARROW_RIGHT));
                        if action_ui(ui, &format!("{salt} {a}"), action, lists, requests) {
                            requests.pick = Some((trigger, r, a));
                        }
                        if ui
                            .add(quiet(CROSS))
                            .on_hover_text("Remove this action")
                            .clicked()
                        {
                            drop = Some(a);
                        }
                    });
                }
                if let Some(a) = drop {
                    rule.then.remove(a);
                }
                ui.menu_button(muted(format!("{PLUS}  Add action")), |ui| {
                    for (label, make) in ACTIONS {
                        if ui.button(*label).clicked() {
                            let action = make(lists, requests);
                            let picks = matches!(action, Action::Teleport(_));
                            rule.then.push(action);
                            if picks {
                                requests.pick = Some((trigger, r, rule.then.len() - 1));
                            }
                            ui.close();
                        }
                    }
                });
            });
    }
    if let Some(r) = remove {
        rules.remove(r);
    }
    if let Some(r) = raise {
        rules.swap(r - 1, r);
    }
    let label = if rules.is_empty() {
        "Add a rule"
    } else {
        "Add another rule (checked if the ones above don't apply)"
    };
    if ui.add(quiet(muted(format!("{PLUS}  {label}")))).clicked() {
        rules.push(Rule {
            when: Condition::Always,
            then: Vec::new(),
        });
    }
}

type MakeAction = fn(&Lists, &mut Requests) -> Action;

const ACTIONS: &[(&str, MakeAction)] = &[
    ("Play a sound", |_, _| Action::Sound(SoundRef(0))),
    ("Show a picture", |_, _| Action::Bubble(SpriteRef(0))),
    ("Show what's missing", |l, r| Action::BubbleMissing {
        item: some_item(l, r),
        count: 1,
    }),
    ("Jump, spin or shake", |_, _| {
        Action::Animate(Animation::Jump)
    }),
    ("Give the player something", |l, r| Action::Give {
        item: some_item(l, r),
        count: 1,
    }),
    ("Take something from the player", |l, r| Action::Take {
        item: some_item(l, r),
        count: 1,
    }),
    ("Set a flag", |l, r| Action::SetFlag(some_flag(l, r))),
    ("Clear a flag", |l, r| Action::ClearFlag(some_flag(l, r))),
    ("Make an object disappear", |l, _| Action::Hide(l.this)),
    ("Follow the player from now on", |_, _| Action::Join),
    ("Take the player somewhere (door)", |l, _| {
        Action::Teleport(Place {
            map: l.maps[0].0,
            at: Pos::new(0, 0),
            facing: Dir::Down,
        })
    }),
];

fn some_item(lists: &Lists, requests: &mut Requests) -> Id {
    lists.items.first().map(|(id, _)| *id).unwrap_or_else(|| {
        requests.new_item = true;
        lists.next
    })
}

fn some_flag(lists: &Lists, requests: &mut Requests) -> Id {
    lists.flags.first().map(|(id, _)| *id).unwrap_or_else(|| {
        requests.new_flag = true;
        lists.next
    })
}

fn default_condition(lists: &Lists, requests: &mut Requests) -> Condition {
    Condition::Flag(some_flag(lists, requests))
}

/// A condition: "Always", "The player has at least …", "Flag … is set" or "… is not set".
fn condition_ui(
    ui: &mut Ui,
    salt: &str,
    condition: &mut Condition,
    lists: &Lists,
    requests: &mut Requests,
    always: bool,
) {
    let kind = match condition {
        Condition::Always => 0,
        Condition::Has { .. } => 1,
        Condition::Flag(_) => 2,
        Condition::NotFlag(_) => 3,
    };
    let labels = [
        "no condition",
        "the player has at least",
        "this flag is set",
        "this flag is not set",
    ];
    // Without a condition, the list offers to add one
    let shown = if kind == 0 {
        "add a condition…"
    } else {
        labels[kind]
    };
    ui.horizontal_wrapped(|ui| {
        let mut chosen = kind;
        ComboBox::from_id_salt(format!("{salt} when"))
            .selected_text(shown)
            .show_ui(ui, |ui| {
                for (k, label) in labels.iter().enumerate() {
                    if k > 0 || always {
                        ui.selectable_value(&mut chosen, k, *label);
                    }
                }
            });
        if chosen != kind {
            *condition = match chosen {
                0 => Condition::Always,
                1 => Condition::Has {
                    item: some_item(lists, requests),
                    count: 1,
                },
                2 => Condition::Flag(some_flag(lists, requests)),
                _ => Condition::NotFlag(some_flag(lists, requests)),
            };
        }
        match condition {
            Condition::Always => {}
            Condition::Has { item, count } => {
                ui.add(egui::DragValue::new(count).range(1..=99));
                item_ui(ui, &format!("{salt} item"), item, lists, requests);
            }
            Condition::Flag(flag) | Condition::NotFlag(flag) => {
                flag_ui(ui, &format!("{salt} flag"), flag, lists, requests);
            }
        }
    });
}

/// One action; true if its "pick on the map" was clicked.
fn action_ui(
    ui: &mut Ui,
    salt: &str,
    action: &mut Action,
    lists: &Lists,
    requests: &mut Requests,
) -> bool {
    let mut pick = false;
    match action {
        Action::Sound(sound) => {
            ui.label("Play");
            choose(ui, salt, &mut sound.0, &lists.sounds);
        }
        Action::Bubble(sprite) => {
            ui.label("Show");
            choose(ui, salt, &mut sprite.0, &lists.sprites);
            ui.label(muted("in a bubble"));
        }
        Action::BubbleMissing { item, count } => {
            ui.label("Show how many are missing of");
            ui.add(egui::DragValue::new(count).range(1..=99));
            item_ui(ui, salt, item, lists, requests);
        }
        Action::Animate(animation) => {
            let text = match animation {
                Animation::Jump => "Jump",
                Animation::Spin => "Spin",
                Animation::Shake => "Shake",
            };
            ComboBox::from_id_salt(salt)
                .selected_text(text)
                .show_ui(ui, |ui| {
                    ui.selectable_value(animation, Animation::Jump, "Jump");
                    ui.selectable_value(animation, Animation::Spin, "Spin");
                    ui.selectable_value(animation, Animation::Shake, "Shake");
                });
        }
        Action::Give { item, count } => {
            ui.label("Give the player");
            ui.add(egui::DragValue::new(count).range(1..=99));
            item_ui(ui, salt, item, lists, requests);
        }
        Action::Take { item, count } => {
            ui.label("Take");
            ui.add(egui::DragValue::new(count).range(1..=99));
            item_ui(ui, salt, item, lists, requests);
        }
        Action::SetFlag(flag) => {
            ui.label("Set");
            flag_ui(ui, salt, flag, lists, requests);
        }
        Action::ClearFlag(flag) => {
            ui.label("Clear");
            flag_ui(ui, salt, flag, lists, requests);
        }
        Action::Hide(thing) => {
            ui.label("Make disappear:");
            let current = if *thing == lists.this {
                "this object".to_string()
            } else {
                Lists::name(&lists.things, *thing)
            };
            ComboBox::from_id_salt(salt)
                .selected_text(current)
                .show_ui(ui, |ui| {
                    ui.selectable_value(thing, lists.this, "this object");
                    for (id, name) in &lists.things {
                        if *id != lists.this {
                            ui.selectable_value(thing, *id, name);
                        }
                    }
                });
        }
        Action::Join => {
            ui.label("Follows the player from now on");
        }
        Action::Teleport(place) => {
            ui.label("Take the player to");
            ComboBox::from_id_salt(salt)
                .selected_text(Lists::name(&lists.maps, place.map))
                .show_ui(ui, |ui| {
                    for (id, name) in &lists.maps {
                        ui.selectable_value(&mut place.map, *id, name);
                    }
                });
            ui.label(muted(format!("at {}, {}", place.at.x, place.at.y)));
            if ui
                .button(format!("{MAP_PIN} Pick on map"))
                .on_hover_text("Click the cell where the player arrives")
                .clicked()
            {
                pick = true;
            }
            ui.label(muted("facing"));
            ComboBox::from_id_salt(format!("{salt} facing"))
                .selected_text(arrow(place.facing))
                .width(40.0)
                .show_ui(ui, |ui| {
                    for dir in Dir::ALL {
                        ui.selectable_value(&mut place.facing, dir, arrow(dir));
                    }
                });
        }
    }
    pick
}

/// Chooses a position in a list of names.
fn choose(ui: &mut Ui, salt: &str, value: &mut u16, names: &[String]) {
    let current = names
        .get(*value as usize)
        .cloned()
        .unwrap_or_else(|| "?".into());
    ComboBox::from_id_salt(salt)
        .selected_text(current)
        .show_ui(ui, |ui| {
            for (i, name) in names.iter().enumerate() {
                ui.selectable_value(value, i as u16, name);
            }
        });
}

fn item_ui(ui: &mut Ui, salt: &str, item: &mut Id, lists: &Lists, requests: &mut Requests) {
    let current = if *item == lists.next {
        "Item (new)".to_string()
    } else {
        Lists::name(&lists.items, *item)
    };
    ComboBox::from_id_salt(format!("{salt} item"))
        .selected_text(current)
        .show_ui(ui, |ui| {
            for (id, name) in &lists.items {
                ui.selectable_value(item, *id, name);
            }
            if ui.button(format!("{PLUS} New item")).clicked() {
                *item = lists.next;
                requests.new_item = true;
                ui.close();
            }
        });
}

fn flag_ui(ui: &mut Ui, salt: &str, flag: &mut Id, lists: &Lists, requests: &mut Requests) {
    let current = if *flag == lists.next {
        "Flag (new)".to_string()
    } else {
        Lists::name(&lists.flags, *flag)
    };
    ComboBox::from_id_salt(format!("{salt} flag"))
        .selected_text(current)
        .show_ui(ui, |ui| {
            for (id, name) in &lists.flags {
                ui.selectable_value(flag, *id, name);
            }
            if ui.button(format!("{PLUS} New flag")).clicked() {
                *flag = lists.next;
                requests.new_flag = true;
                ui.close();
            }
        });
}

/// The world settings' section for items and flags.
pub(crate) fn items_and_flags(
    ui: &mut Ui,
    project: &mut Project,
    pictures: &[Option<(TextureHandle, Rect)>],
) {
    let used = |project: &Project, id: Id| match use_count(project, id) {
        0 => "not used yet".to_string(),
        1 => "used by 1 rule".to_string(),
        n => format!("used by {n} rules"),
    };
    field(
        ui,
        "Items · what the player can carry, shown as icons on the device",
        |ui| {
            let counts: Vec<(usize, String)> = project
                .items
                .iter()
                .map(|item| (use_count(project, item.id), used(project, item.id)))
                .collect();
            let mut remove = None;
            let width = ui.available_width();
            for (i, item) in project.items.iter_mut().enumerate() {
                let (count, used) = &counts[i];
                card(ui, width, |ui| {
                    // The item's picture: click it to choose another
                    let (rect, response) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::click());
                    let response = response
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .on_hover_text("Choose its picture");
                    ui.painter().rect_filled(rect, 10, PANEL);
                    if let Some(Some((texture, uv))) = pictures.get(item.icon.0 as usize) {
                        picture(ui, texture, *uv, rect.shrink(4.0));
                    }
                    if response.hovered() {
                        ui.painter().rect_stroke(
                            rect,
                            10,
                            egui::Stroke::new(1.5, MUTED),
                            egui::StrokeKind::Outside,
                        );
                    }
                    egui::Popup::from_toggle_button_response(&response)
                        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                        .show(|ui| {
                            ui.label(muted("Its picture:"));
                            sprite_grid(ui, pictures, &mut item.icon, 240.0, true);
                        });
                    name_and_use(
                        ui,
                        &mut item.name,
                        "the item's name",
                        used,
                        *count,
                        &mut remove,
                        i,
                    );
                });
            }
            if let Some(i) = remove {
                project.items.remove(i);
            }
            if ui.add(quiet(muted(format!("{PLUS}  New item")))).clicked() {
                let id = project.new_id();
                let name = format!("Item {}", project.items.len() + 1);
                project.items.push(ItemDef {
                    id,
                    name,
                    icon: SpriteRef(0),
                });
            }
        },
    );
    ui.add_space(8.0);
    field(
        ui,
        "Flags · remember that something happened, e.g. \"gate open\"",
        |ui| {
            let counts: Vec<(usize, String)> = project
                .flags
                .iter()
                .map(|flag| (use_count(project, flag.id), used(project, flag.id)))
                .collect();
            let mut remove = None;
            let width = ui.available_width();
            for (i, flag) in project.flags.iter_mut().enumerate() {
                let (count, used) = &counts[i];
                card(ui, width, |ui| {
                    let (rect, _) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::hover());
                    ui.painter().rect_filled(rect, 10, PANEL);
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        FLAG,
                        egui::FontId::proportional(22.0),
                        MUTED,
                    );
                    name_and_use(
                        ui,
                        &mut flag.name,
                        "the flag's name",
                        used,
                        *count,
                        &mut remove,
                        i,
                    );
                });
            }
            if let Some(i) = remove {
                project.flags.remove(i);
            }
            if ui.add(quiet(muted(format!("{PLUS}  New flag")))).clicked() {
                let id = project.new_id();
                let name = format!("Flag {}", project.flags.len() + 1);
                project.flags.push(FlagDef { id, name });
            }
        },
    );
}

/// A row in a list of items or flags, `width` wide: measured once for the whole list, as each
/// row would otherwise widen the column for the next.
pub(crate) fn card(ui: &mut Ui, width: f32, add: impl FnOnce(&mut Ui)) {
    Frame::new()
        .fill(PANEL2.gamma_multiply(1.4))
        .corner_radius(12)
        .inner_margin(Margin::same(8))
        .show(ui, |ui| {
            ui.set_width(width - 16.0);
            ui.horizontal(add);
        });
    ui.add_space(4.0);
}

/// The name to type in, how often it's used, and a delete button while it isn't used.
pub(crate) fn name_and_use(
    ui: &mut Ui,
    name: &mut String,
    hint: &str,
    used: &str,
    count: usize,
    remove: &mut Option<usize>,
    index: usize,
) {
    // The button first, from the right, so the name takes exactly the rest: a guessed width
    // made the card wider than its column, which then widened everything after it
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        let delete = ui
            .add_enabled(count == 0, quiet(RichText::new(TRASH).size(16.0)))
            .on_hover_text("Delete")
            .on_disabled_hover_text("Still in use, so it can't be deleted");
        if delete.clicked() {
            *remove = Some(index);
        }
        ui.with_layout(Layout::top_down(Align::Min), |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            text_input(ui, name, hint);
            ui.label(muted(used).small());
        });
    });
}

/// All sprites as pictures to pick one from; closes the popup it's in, if asked.
pub(crate) fn sprite_grid(
    ui: &mut Ui,
    pictures: &[Option<(TextureHandle, Rect)>],
    chosen: &mut SpriteRef,
    width: f32,
    in_popup: bool,
) {
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(width);
        for (i, entry) in pictures.iter().enumerate() {
            let (rect, response) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::click());
            let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
            ui.painter().rect_filled(rect, 8, PANEL);
            if let Some((texture, uv)) = entry {
                picture(ui, texture, *uv, rect.shrink(4.0));
            }
            if chosen.0 as usize == i {
                ring(ui, rect, 10);
            } else if response.hovered() {
                ui.painter().rect_stroke(
                    rect,
                    8,
                    egui::Stroke::new(1.5, MUTED),
                    egui::StrokeKind::Outside,
                );
            }
            if response.clicked() {
                *chosen = SpriteRef(i as u16);
                if in_popup {
                    ui.close();
                }
            }
        }
    });
}

/// How many rules, conditions and tile changes refer to an item or flag.
fn use_count(project: &Project, id: Id) -> usize {
    let condition = |c: &Condition| match c {
        Condition::Has { item, .. } => *item == id,
        Condition::Flag(f) | Condition::NotFlag(f) => *f == id,
        Condition::Always => false,
    };
    let action = |a: &Action| match a {
        Action::BubbleMissing { item, .. }
        | Action::Give { item, .. }
        | Action::Take { item, .. } => *item == id,
        Action::SetFlag(f) | Action::ClearFlag(f) => *f == id,
        _ => false,
    };
    project
        .maps
        .iter()
        .map(|map| {
            let swaps = map.swaps.iter().filter(|s| s.when == id).count();
            let rules: usize = map
                .things
                .iter()
                .map(|t| {
                    let shown = condition(&t.thing.shown_if) as usize;
                    let rules = t.thing.on_bump.iter().chain(&t.thing.on_step);
                    shown
                        + rules
                            .filter(|r| condition(&r.when) || r.then.iter().any(action))
                            .count()
                })
                .sum();
            swaps + rules
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::starter;

    #[test]
    fn counts_what_rules_use() {
        let (mut project, _) = starter();
        let apple = project.items[0].id;
        let gate = project.flags[0].id;
        // Four apples give one, the horse checks and takes them, grandma gives one
        assert_eq!(use_count(&project, apple), 4 + 2 + 1);
        // The horse checks and sets it, the gate's tile changes with it
        assert_eq!(use_count(&project, gate), 2 + 1);
        let id = project.new_id();
        project.flags.push(FlagDef {
            id,
            name: "Unused".into(),
        });
        assert_eq!(use_count(&project, id), 0);
    }
}
