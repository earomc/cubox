use std::sync::Arc;

use anyhow::Ok;
use winit::window::Window;

struct App {
    state: Option<State>
}

impl App {
    fn new() -> Self {
        Self { state: None }
    }
}

struct State {
    window: Arc<Window>,
}

impl State {
    pub async fn new(window: Arc<Window>) -> anyhow::Result<Self> {
        Ok(Self { window })
    }

    pub fn resize(&mut self, _width: u32, _height: u32) {
        // 
    }

    pub fn render(&mut self) {
        self.window.request_redraw();
    }
}

pub fn run() {
    
}