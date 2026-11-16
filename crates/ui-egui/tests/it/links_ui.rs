//! Headless UI test of the community links: Help menu commands, the header Discord button, the
//! About dialog and the Home (Import) screen.
//!
//! Set `FILMCRAFT_UI_SNAPSHOT_DIR=<dir>` to also render the window offscreen with wgpu and write
//! `links-*.png` there; without it no GPU is needed.

use std::sync::mpsc::{Sender, channel};

use egui_kittest::Harness;
use filmcraft_engine::Session;
use filmcraft_ui_egui::FilmcraftApp;
use filmcraft_ui_egui::control::ControlRequest;
use filmcraft_ui_egui::links;
use serde_json::{Value, json};

struct Driver {
    harness: Harness<'static, FilmcraftApp>,
    tx: Sender<ControlRequest>,
    snapshots: Option<std::path::PathBuf>,
}

impl Driver {
    fn demo() -> Self {
        let mut session = Session::default();
        session.execute("file.openDemoProject", json!({})).expect("demo project");
        let (tx, rx) = channel();
        let app = FilmcraftApp::new(session).with_control(rx);
        let snapshots = std::env::var_os("FILMCRAFT_UI_SNAPSHOT_DIR").map(std::path::PathBuf::from);
        let mut b = Harness::builder().with_size(egui::vec2(1600.0, 980.0)).with_max_steps(10_000);
        if snapshots.is_some() {
            b = b.wgpu();
        }
        let harness = b.build_eframe(move |_cc| app);
        let mut d = Driver { harness, tx, snapshots };
        d.frames(4);
        d
    }

    fn frames(&mut self, n: usize) {
        for _ in 0..n {
            let ctx = self.harness.ctx.clone();
            let mut raw = std::mem::take(self.harness.input_mut());
            eframe::App::raw_input_hook(self.harness.state_mut(), &ctx, &mut raw);
            *self.harness.input_mut() = raw;
            self.harness.step();
        }
    }

    fn ok(&mut self, method: &str, params: Value) -> Value {
        let (req, reply) = ControlRequest::new(method, params.clone());
        self.tx.send(req).unwrap();
        for _ in 0..600 {
            self.frames(1);
            if let Ok(v) = reply.try_recv() {
                assert_eq!(v["ok"], json!(true), "{method} {params} failed: {v}");
                return v["result"].clone();
            }
        }
        panic!("no reply to {method} {params}");
    }

    fn ids(&mut self, prefix: &str) -> Vec<String> {
        let v = self.ok("ui.elements", json!({"prefix": prefix}));
        v.as_array().unwrap().iter().filter_map(|e| e["id"].as_str().map(str::to_string)).collect()
    }

    fn snapshot(&mut self, name: &str) {
        let Some(dir) = self.snapshots.clone() else { return };
        self.frames(2);
        match self.harness.render() {
            Ok(img) => {
                std::fs::create_dir_all(&dir).unwrap();
                img.save(dir.join(format!("links-{name}.png"))).unwrap();
            }
            Err(e) => eprintln!("snapshot {name} skipped: {e}"),
        }
    }
}

#[test]
fn community_links_everywhere() {
    let mut d = Driver::demo();
    // Help menu: every link is a command that reports the URL it opened.
    for (id, _, url) in links::ALL {
        let r = d.ok("engine.execute", json!({"command": id, "params": {}}));
        assert_eq!(r["url"], json!(url), "{id}");
    }
    let menu = d.ok("ui.menu.list", json!({}));
    let text = menu.to_string();
    for (id, label, _) in links::ALL {
        assert!(text.contains(id) && text.contains(label), "Help menu lists {id}");
    }
    // Header: the Discord button is visible when there is room.
    assert_eq!(d.ids("header.discord"), vec!["header.discord".to_string()]);
    d.snapshot("header");
    // About dialog: Discord first, then website, app page, GitHub, issues.
    d.ok("engine.execute", json!({"command": "app.about", "params": {}}));
    d.frames(3);
    let about = d.ids("about.");
    for id in ["about.discord", "about.website", "about.appPage", "about.github", "about.reportIssue"] {
        assert!(about.iter().any(|a| a == id), "{id} in {about:?}");
    }
    d.snapshot("about");
    d.ok("ui.key", json!({"key": "Escape"}));
    d.frames(3);
    assert!(d.ids("about.").is_empty(), "Escape closes About");
    // Home screen (Import mode): community links.
    d.ok("engine.execute", json!({"command": "mode.import", "params": {}}));
    d.frames(3);
    let home = d.ids("import.link.");
    for id in ["import.link.discord", "import.link.website", "import.link.appPage", "import.link.github"] {
        assert!(home.iter().any(|a| a == id), "{id} in {home:?}");
    }
    d.snapshot("home");
}

#[test]
fn header_controls_do_not_overlap_menus_when_resized() {
    let mut d = Driver::demo();
    let mut previous = Vec::new();
    let mut intermediate_counts = std::collections::BTreeSet::new();
    for width in (800..=1600).step_by(20) {
        let width = width as f32;
        d.harness.set_size(egui::vec2(width, 980.0));
        d.frames(4);
        let menus = d.ok("ui.elements", json!({"prefix": "menu."}));
        let header = d.ok("ui.elements", json!({"prefix": "header."}));
        let menus = menus.as_array().unwrap();
        let header = header.as_array().unwrap();
        let right_ids = [
            "header.discord",
            "header.workspaceName",
            "header.workspaces",
            "header.appearance",
            "header.quickExport",
            "header.notifications",
            "header.search",
            "header.volume",
            "header.fullscreen",
        ];
        let priority = [
            "header.workspaces",
            "header.search",
            "header.quickExport",
            "header.notifications",
            "header.fullscreen",
            "header.volume",
            "header.appearance",
            "header.workspaceName",
            "header.discord",
        ];
        let visible: Vec<_> = priority.iter().filter(|id| header.iter().any(|e| e["id"] == **id)).copied().collect();
        if width == 960.0 {
            assert!(!visible.is_empty() && visible.len() < priority.len(), "half width should retain some controls: {visible:?}");
        }
        assert_eq!(visible, priority[..visible.len()], "priority at width {width}");
        assert!(previous.iter().all(|id| visible.contains(id)), "controls disappeared while widening at {width}");
        if !visible.is_empty() && visible.len() < priority.len() {
            intermediate_counts.insert(visible.len());
        }
        previous = visible;
        let controls: Vec<_> = header.iter().filter(|e| right_ids.contains(&e["id"].as_str().unwrap_or(""))).collect();
        for (index, control) in controls.iter().enumerate() {
            let left = control["rect"][0].as_f64().unwrap();
            let right = left + control["rect"][2].as_f64().unwrap();
            for other in controls.iter().skip(index + 1) {
                let other_left = other["rect"][0].as_f64().unwrap();
                let other_right = other_left + other["rect"][2].as_f64().unwrap();
                assert!(right <= other_left || other_right <= left, "width {width}: {control} overlaps {other}");
            }
            for menu in menus {
                let rect = &menu["rect"];
                let right = rect[0].as_f64().unwrap() + rect[2].as_f64().unwrap();
                assert!(left >= right, "width {width}: {control} overlaps {menu}");
            }
        }
    }
    assert!(intermediate_counts.len() >= 3, "controls should disappear individually");
    assert_eq!(previous.len(), 9, "all controls return at full width");
}
