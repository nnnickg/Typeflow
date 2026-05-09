use typeflow_core::{Engine, EngineConfig, InputEvent, Layout};
use typeflow_core::data::LanguageBundle;

fn main() {
    let bundle = LanguageBundle::embedded().unwrap();
    let mut engine = Engine::new(EngineConfig::default(), bundle);
    engine.reset_layout(Layout::English);
    
    // Type 'io' (which is 'що' in Ukrainian)
    let e1 = engine.input_event_from_char('i');
    let e2 = engine.input_event_from_char('o');
    let space = engine.input_event_from_char(' ');
    
    let a1 = engine.process(e1);
    let a2 = engine.process(e2);
    let a3 = engine.process(space);
    
    println!("Action 1: {:?}", a1.action);
    println!("Action 2: {:?}", a2.action);
    println!("Action 3: {:?}", a3.action);
}
