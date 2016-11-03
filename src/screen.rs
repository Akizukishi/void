use std::cmp;
use std::fs::{File, rename, remove_file};
use std::collections::BTreeMap;
use std::io::{Write, Stdout, stdout, stdin};
use std::cell::RefCell;
use std::rc::Rc;
use std::process::exit;

use termion;
use termion::event::{Key, Event, MouseEvent};
use termion::input::{TermRead, MouseTerminal};
use termion::raw::{IntoRawMode, RawTerminal};

use {NodeRef, Coords, Node, Content, Meta};
use serialization;
use logging;

#[derive(Clone)]
struct NodeLookup {
    // anchor of selected node
    anchor: NodeRef,
    // selected node
    node: NodeRef,
}

pub struct Screen {
    pub anchors: BTreeMap<Coords, NodeRef>,
    arrows: Vec<(NodeLookup, NodeLookup)>,
    last_selected: Option<NodeLookup>,
    stdout: Option<MouseTerminal<RawTerminal<Stdout>>>,
    dragging_from: Option<Coords>,
    drawing_arrow: Option<NodeLookup>,
    pub work_path: Option<String>,
    pub max_id: u64,
}

impl Default for Screen {
    fn default() -> Screen {
        Screen {
            anchors: BTreeMap::new(),
            arrows: vec![],
            last_selected: None,
            stdout: None,
            dragging_from: None,
            drawing_arrow: None,
            work_path: None,
            max_id: 0,
        }
    }
}

impl Screen {
    fn handle_event(&mut self, evt: Event) {
        match evt {
            Event::Key(Key::Char('\n')) => self.toggle_collapsed(),
            Event::Key(Key::Char('\t')) => self.create_child(),
            Event::Key(Key::Delete) => self.delete_selected(),
            Event::Key(Key::Ctrl('f')) => self.toggle_hide_stricken(),
            Event::Key(Key::Ctrl('x')) => self.toggle_stricken(),
            // Event::Key(Key::Alt('\u{1b}')) |
            Event::Key(Key::Ctrl('a')) => self.draw_arrow(),
            Event::Key(Key::Ctrl('c')) |
            Event::Key(Key::Ctrl('d')) => self.exit(),
            Event::Key(Key::Ctrl('s')) |
            Event::Key(Key::Ctrl('w')) => self.save(),
            Event::Key(Key::Up) => self.select_up(),
            Event::Key(Key::Down) => self.select_down(),
            Event::Key(Key::Backspace) => self.backspace(),
            Event::Key(Key::Char(c)) => self.append(c),
            Event::Mouse(me) => {
                match me {
                    MouseEvent::Press(_, x, y) => self.click((x, y)),
                    MouseEvent::Release(x, y) => {
                        self.release((x, y));
                    }
                    MouseEvent::Hold(..) => {
                        // this isn't supported in some terminals
                        // (urxvt...) so don't rely on it
                    }
                }
            }
            e => warn!("Weird event {:?}", e),
        }
    }

    fn draw(&mut self) {
        // clear screen
        print!("\x1b[2J\x1b[H");

        for (coords, anchor) in &self.anchors {
            anchor.borrow().draw_tree("".to_string(), coords.0, coords.1, false);
        }

        // print logs
        let (width, bottom) = termion::terminal_size().unwrap();
        if width > 4 && bottom > 7 {
            let mut sep = format!("{}{}logs{}",
                                  termion::cursor::Goto(0, bottom - 6),
                                  termion::style::Invert,
                                  termion::style::Reset);
            for _ in 0..width - 4 {
                sep.push('█');
            }
            println!("{}{}", termion::cursor::Goto(0, bottom - 12), sep);
            {
                let logs = logging::read_logs();
                for msg in logs.iter().rev() {
                    println!("\r{}", msg);
                }
            }
        }

        // let p = self.path((6, 16), (6, 22));
        // self.draw_path(p);

        for &(ref from, ref to) in &self.arrows {
            let path = self.path_between_nodes(from.clone(), to.clone());
            self.draw_path(path);
        }


        print!("{}", termion::cursor::Hide);
        if let Some(mut s) = self.stdout.take() {
            s.flush().unwrap();
            self.stdout = Some(s);
        }
    }

    fn insert(&mut self, coords: Coords, node: Node) {
        let safe_coords = (cmp::max(coords.0, 1), cmp::max(coords.1, 1));
        self.anchors.insert(safe_coords, Rc::new(RefCell::new(node)));
    }

    fn coords_for_anchor(&self, node: &NodeRef) -> Option<Coords> {
        // if we switch to screen as grid of refs, use that instead
        for (&coords, anchor) in &self.anchors {
            if anchor.as_ptr() == node.as_ptr() {
                return Some(coords);
            }
        }
        None
    }

    fn path_between_nodes(&self, start: NodeLookup, to: NodeLookup) -> Vec<Coords> {
        let (s1, s2) = self.bounds_for_lookup(start).unwrap();
        let (t1, t2) = self.bounds_for_lookup(to).unwrap();

        let init = self.path(s1, t1);
        let paths = vec![
            self.path(s1, t2),
            self.path(s2, t1),
            self.path(s2, t2),
        ];
        paths.into_iter()
            .fold(init, |short, path| {
                if path.len() < short.len() {
                    path
                } else {
                    short
                }
            })
    }

    fn bounds_for_lookup(&self, lookup: NodeLookup) -> Option<(Coords, Coords)> {
        if let Some(left) = self.coords_for_lookup(lookup.clone()) {
            let mut rx = left.0;
            let node_ptr = lookup.node.as_ptr();
            while let Some(cursor) = self.find_child_at_coords((rx, left.1)) {
                if cursor.node.as_ptr() == node_ptr {
                    rx += 1;
                } else {
                    break;
                }
            }
            let right = (rx, left.1);
            Some((left, right))
        } else {
            None
        }
    }

    fn coords_for_lookup(&self, lookup: NodeLookup) -> Option<Coords> {
        // if we switch to screen as grid of refs, use that instead
        // possible that a parent / anchor has been deleted
        self.coords_for_anchor(&lookup.anchor).map(|(anchor_x, anchor_y)| {
            let anchor_children = lookup.anchor.borrow().flat_visible_children();
            let mut idx = 0;
            for (i, child) in anchor_children.iter().enumerate() {
                if child.as_ptr() == lookup.node.as_ptr() {
                    idx = i + 1;
                }
            }
            (anchor_x, anchor_y + idx as u16)
        })
    }

    fn find_child_at_coords(&self, coords: Coords) -> Option<NodeLookup> {
        // scan possible anchors
        let mut candidate_anchors = vec![];
        for (&(x, y), anchor) in &self.anchors {
            if coords.0 >= x && coords.1 >= y && coords.1 - y < anchor.borrow().height() as u16 {
                candidate_anchors.push(((x, y), anchor.clone()));
            }
        }
        // scan possible nodes
        let mut candidate_nodes = vec![];
        for ((x, y), anchor) in candidate_anchors {
            let lookup_coords = (coords.0 - x, coords.1 - y);
            let look = if lookup_coords.1 == 0 {
                if anchor.borrow().content.len() + 1 >= lookup_coords.0 as usize {
                    Some(anchor.clone())
                } else {
                    None
                }
            } else {
                anchor.borrow().find_child_at_coords(0, lookup_coords)
            };
            if let Some(node) = look {
                candidate_nodes.push(NodeLookup {
                    anchor: anchor.clone(),
                    node: node,
                });
            }
        }
        candidate_nodes.pop()
    }

    fn pop_selected(&mut self) -> Option<NodeLookup> {
        if self.dragging_from.is_none() {
            if let Some(lookup) = self.last_selected.take() {
                lookup.node.borrow_mut().selected = false;
                Some(lookup.clone())
            } else {
                None
            }
        } else {
            None
        }
    }

    fn try_select(&mut self, coords: Coords) -> Option<NodeLookup> {
        if self.dragging_from.is_none() {
            if let Some(ref lookup) = self.find_child_at_coords(coords) {
                lookup.node.borrow_mut().selected = true;
                self.last_selected = Some(lookup.clone());
                self.dragging_from = Some(coords);
                Some(lookup.clone())
            } else {
                None
            }
        } else {
            None
        }
    }

    fn toggle_stricken(&mut self) {
        if let Some(ref lookup) = self.last_selected {
            let mut node = lookup.node.borrow_mut();
            node.toggle_stricken();
        }
    }

    fn toggle_hide_stricken(&mut self) {
        if let Some(ref lookup) = self.last_selected {
            let mut node = lookup.node.borrow_mut();
            node.toggle_hide_stricken();
        }
    }

    fn delete_selected(&mut self) {
        if let Some(lookup) = self.last_selected.take() {
            let coords = self.coords_for_lookup(lookup.clone());
            let ptr = {
                lookup.anchor.as_ptr()
            };
            if ptr == lookup.node.as_ptr() {
                // nuke whole anchor
                let anchors = self.anchors
                    .clone()
                    .into_iter()
                    .filter(|&(_, ref anchor)| anchor.as_ptr() != ptr)
                    .collect();
                self.anchors = anchors;
            } else {
                lookup.anchor.borrow_mut().delete(lookup.node.clone());
            }
            if let Some(c) = coords {
                self.click_select(c);
            }
        }
    }

    fn create_child(&mut self) {
        if let Some(ref mut lookup) = self.last_selected.clone() {
            let child = lookup.node.borrow_mut().create_child();
            let new_lookup = NodeLookup {
                anchor: lookup.anchor.clone(),
                node: child,
            };
            self.select_node(new_lookup);
        }
    }

    pub fn run(&mut self) {
        if self.stdout.is_none() {
            self.stdout = Some(MouseTerminal::from(stdout().into_raw_mode().unwrap()));
        }
        self.draw();
        let stdin = stdin();
        for c in stdin.events() {
            let evt = c.unwrap();
            self.handle_event(evt);
            self.draw();
        }
    }

    fn toggle_collapsed(&mut self) {
        if let Some(ref lookup) = self.last_selected {
            lookup.node.borrow_mut().toggle_collapsed()
        }
    }

    fn create_anchor(&mut self, coords: Coords) {
        let node = Node {
            content: Content::Text { text: "".to_string() },
            children: vec![],
            selected: false,
            collapsed: false,
            stricken: false,
            hide_stricken: false,
            meta: Meta::default(), // TODO do this forreal
        };
        self.insert(coords, node);
    }

    fn backspace(&mut self) {
        if let Some(ref lookup) = self.last_selected {
            let mut node = lookup.node.borrow_mut();
            node.content.backspace();
        }
    }

    fn append(&mut self, c: char) {
        if let Some(ref lookup) = self.last_selected {
            let mut node = lookup.node.borrow_mut();
            node.content.append(c);
        }
    }

    fn move_selected(&mut self, from: Coords, to: Coords) {
        let dx = to.0 as i16 - from.0 as i16;
        let dy = to.1 as i16 - from.1 as i16;

        let anchors_clone = self.anchors.clone();
        if let Some(ref lookup) = self.last_selected {
            for (coords, value) in &anchors_clone {
                let nx = cmp::max(coords.0 as i16 + dx, 1) as u16;
                let ny = cmp::max(coords.1 as i16 + dy, 1) as u16;
                if value.as_ptr() == lookup.anchor.as_ptr() {
                    let anchor = self.anchors.remove(coords).unwrap();
                    self.anchors.insert((nx, ny), anchor);
                }
            }
        }
    }

    fn click_select(&mut self, coords: Coords) -> Option<NodeLookup> {
        self.pop_selected();
        let result = self.try_select((coords.0, coords.1));
        self.dragging_from.take();
        result
    }

    fn select_up(&mut self) {
        if let Some(lookup) = self.last_selected.clone() {
            if let Some(coords) = self.coords_for_lookup(lookup) {
                // to prevent selection fall-off, click old coords
                // if nothing is selected above this node
                if coords.1 > 0 {
                    self.click_select((coords.0, coords.1 - 1))
                        .or_else(|| self.click_select(coords));
                }
            }
        }
    }

    fn select_down(&mut self) {
        if let Some(lookup) = self.last_selected.clone() {
            if let Some(coords) = self.coords_for_lookup(lookup) {
                // to prevent selection fall-off, click old coords
                // if nothing is selected below this node
                self.click_select((coords.0, coords.1 + 1))
                    .or_else(|| self.click_select(coords));
            }
        }
    }

    fn select_node(&mut self, lookup: NodeLookup) {
        self.pop_selected();
        let mut node = lookup.node.borrow_mut();
        node.selected = true;
        self.last_selected = Some(lookup.clone());
    }

    fn draw_arrow(&mut self) {
        if let Some(from) = self.drawing_arrow.take() {
            self.last_selected.clone().map(|to| self.arrows.push((from, to)));
        } else {
            self.drawing_arrow = self.last_selected.clone();
        }
    }

    fn click(&mut self, coords: Coords) {
        let (x, y) = coords;
        let old = self.pop_selected();
        self.try_select((x, y));
        if old.is_none() && self.dragging_from.is_none() {
            self.create_anchor((x, y));
        }
    }

    fn release(&mut self, coords: Coords) {
        let (x, y) = coords;
        if let Some((from_x, from_y)) = self.dragging_from.take() {
            self.move_selected((from_x, from_y), (x, y));
        }
    }

    fn save(&self) {
        let data = serialization::serialize_screen(self);
        if let Some(ref path) = self.work_path {
            let mut tmp_path = path.clone();
            tmp_path.push_str(".tmp");
            if let Ok(_) = remove_file(&tmp_path) {
                debug!("removed stale tmp file");
            }
            let mut f = File::create(&tmp_path).unwrap();
            f.write_all(&*data).unwrap();
            rename(tmp_path, path).unwrap();
            info!("saved work to {}", path);
        }
    }

    fn exit(&mut self) {
        let (_, bottom) = termion::terminal_size().unwrap();
        print!("{}", termion::cursor::Goto(0, bottom));
        println!("{}", termion::cursor::Show);
        self.stdout.take().unwrap().flush().unwrap();
        self.save();
        exit(0);
    }

    fn occupied(&self, coords: Coords) -> bool {
        self.find_child_at_coords(coords).is_some()
    }

    fn path(&self, start: Coords, dest: Coords) -> Vec<Coords> {
        fn cost(c1: Coords, c2: Coords) -> u16 {
            let xcost = cmp::max(c1.0, c2.0) - cmp::min(c1.0, c2.0);
            let ycost = cmp::max(c1.1, c2.1) - cmp::min(c1.1, c2.1);
            xcost + ycost
        }
        fn perms(c: Coords) -> Vec<Coords> {
            vec![(c.0 + 1, c.1),
                 (cmp::max(c.0, 1) - 1, c.1),
                 (c.0, c.1 + 1),
                 (c.0, cmp::max(c.1, 1) - 1)]
        }
        // maps from location to previous location
        let mut visited: BTreeMap<Coords, Coords> = BTreeMap::new();
        let mut pq = PrioQueue::default();

        let mut cursor = start;
        while cursor != dest {
            for neighbor in perms(cursor) {
                if (!self.occupied(neighbor) || neighbor == dest) &&
                   !visited.contains_key(&neighbor) {
                    let c = cost(neighbor, dest);
                    pq.insert(c, neighbor);
                    visited.insert(neighbor, cursor);
                }
            }
            cursor = pq.pop().unwrap();
            // self.draw_path(visited.clone().keys().map(|k| *k).collect());

        }
        let mut back_cursor = dest;
        let mut path = vec![];
        while back_cursor != start {
            let prev = visited.get(&back_cursor).unwrap();
            path.push(*prev);
            back_cursor = *prev;
        }
        path.reverse();
        path
    }

    fn draw_path(&self, path: Vec<Coords>) {
        // print!("{}", termion::color::Fg(termion::color::Green));
        for coords in &path {
            print!("{}*", termion::cursor::Goto(coords.0, coords.1))
        }
        print!("{}", termion::color::Fg(termion::color::Reset));
    }
}

struct PrioQueue {
    to_visit: BTreeMap<u16, Vec<Coords>>,
}

impl Default for PrioQueue {
    fn default() -> PrioQueue {
        PrioQueue { to_visit: BTreeMap::new() }
    }
}

impl PrioQueue {
    fn insert(&mut self, k: u16, v: Coords) {
        let mut cur = self.to_visit.remove(&k).unwrap_or_else(|| vec![]);
        cur.push(v);
        self.to_visit.insert(k, cur);
    }
    fn pop(&mut self) -> Option<Coords> {
        if let Some((lowest_cost, _)) = self.to_visit.clone().iter().nth(0) {
            let mut cur = self.to_visit.remove(lowest_cost).unwrap_or_else(|| vec![]);
            let coords = cur.pop();
            if !cur.is_empty() {
                self.to_visit.insert(*lowest_cost, cur);
            }
            coords
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use termion::event::{Key, Event, MouseEvent, MouseButton};

    use quickcheck::{Arbitrary, Gen, QuickCheck, StdGen};
    use rand;

    use super::*;

    #[derive(Debug, Clone)]
    struct Op {
        event: Event,
    }

    impl Arbitrary for Op {
        fn arbitrary<G: Gen>(g: &mut G) -> Op {
            let (c, x, y) = (g.gen::<char>(), g.gen::<u16>(), g.gen::<u16>());
            let events = vec![
                Event::Key(Key::Char(c)),
                Event::Key(Key::Alt('\u{1b}')),
                Event::Key(Key::Ctrl(c)),
                Event::Key(Key::Up),
                Event::Key(Key::Down),
                Event::Key(Key::Backspace),
                Event::Mouse(MouseEvent::Press(MouseButton::Left, x, y)),
                Event::Mouse(MouseEvent::Release(x, y)),
                Event::Mouse(MouseEvent::Hold(x, y)),
            ];
            Op { event: *g.choose(&*events).unwrap() }
        }
    }


    #[derive(Debug, Clone)]
    struct OpVec {
        ops: Vec<Op>,
    }

    impl Arbitrary for OpVec {
        fn arbitrary<G: Gen>(g: &mut G) -> OpVec {
            let mut ops = vec![];
            for _ in 0..g.gen_range(1, 100) {
                ops.push(Op::arbitrary(g));
            }
            OpVec { ops: ops }
        }

        fn shrink(&self) -> Box<Iterator<Item = OpVec>> {
            let mut smaller = vec![];
            for i in 0..self.ops.len() {
                let mut clone = self.clone();
                clone.ops.remove(i);
                smaller.push(clone);
            }

            Box::new(smaller.into_iter())
        }
    }

    fn prop_handle_events(ops: OpVec) -> bool {
        let mut screen = Screen::default();
        for op in &ops.ops {
            screen.handle_event(op.event);
            screen.draw();
        }
        true
    }

    #[test]
    // #[ignore]
    fn qc_merge_converges() {
        QuickCheck::new()
            .gen(StdGen::new(rand::thread_rng(), 1))
            .tests(1_000)
            .max_tests(10_000)
            .quickcheck(prop_handle_events as fn(OpVec) -> bool);
    }
}
