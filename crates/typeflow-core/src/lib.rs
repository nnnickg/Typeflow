#![cfg_attr(test, allow(clippy::expect_used, clippy::panic, clippy::unwrap_used))]

pub mod data;
mod engine;
mod keyboard;
mod score;
mod types;

pub use engine::{Engine, is_literal_bypass_char};
pub use keyboard::{
    KeyboardMap, KeyboardMapError, LetterEvent, PhysicalKey, render_candidates_with_bundle,
    render_letters_with_bundle,
};
pub use score::has_dictionary_evidence;
pub use types::{
    CompositionAction, CompositionOutput, Decision, EngineConfig, EngineConfigError, HostContext,
    InputEvent, Layout, LayoutCandidates, LayoutScore, MAX_CONFIG_TOKEN_LEN, ScoreAnalysis,
};

#[cfg(test)]
mod tests {
    use super::{
        CompositionAction, Decision, Engine, EngineConfig, EngineConfigError, HostContext,
        InputEvent, KeyboardMap, Layout, LetterEvent, MAX_CONFIG_TOKEN_LEN, PhysicalKey,
        ScoreAnalysis,
    };
    use crate::data::LanguageBundle;
    use proptest::prelude::*;

    fn fixture_bundle() -> LanguageBundle {
        LanguageBundle::for_testing(
            &[
                ("hello", 1000),
                ("typeflow", 50),
                ("type", 800),
                ("flow", 600),
                ("the", 5000),
                ("and", 4000),
                ("input", 700),
                ("language", 300),
                ("http", 2000),
                ("https", 1800),
                ("json", 1700),
                ("aws", 1600),
                ("kubectl", 1500),
                ("terraform", 1400),
                ("token", 1300),
                ("secret", 1200),
                ("password", 1100),
                ("bearer", 1000),
                ("namespace", 900),
                ("deployment", 800),
            ],
            &[
                ("привіт", 900),
                ("приватний", 200),
                ("звичка", 150),
                ("мир", 1000),
                ("мова", 600),
                ("баба", 500),
                ("розкладка", 100),
                ("клавіатура", 80),
                ("перемикання", 70),
            ],
        )
    }

    fn engine() -> Engine {
        Engine::new(EngineConfig::default(), fixture_bundle())
    }

    fn engine_with_config(config: EngineConfig) -> Engine {
        Engine::new(config, fixture_bundle())
    }

    fn physical_key_strategy() -> impl Strategy<Value = PhysicalKey> {
        (0u8..PhysicalKey::COUNT as u8).prop_map(|index| {
            PhysicalKey::from_index(index).expect("generated physical key index must be valid")
        })
    }

    fn letter_event_strategy() -> impl Strategy<Value = LetterEvent> {
        (physical_key_strategy(), any::<bool>()).prop_map(|(physical_key, shift)| LetterEvent {
            physical_key,
            shift,
        })
    }

    fn input_event_strategy() -> impl Strategy<Value = InputEvent> {
        prop_oneof![
            8 => letter_event_strategy().prop_map(InputEvent::Letter),
            3 => any::<char>().prop_map(InputEvent::Literal),
            2 => Just(InputEvent::Backspace),
            2 => Just(InputEvent::EndToken),
            1 => Just(InputEvent::HostBypass),
        ]
    }

    #[test]
    fn it_defaults_to_english() {
        let engine = engine();
        assert_eq!(engine.current_layout(), Layout::English);
        assert_eq!(engine.token_len(), 0);
        assert_eq!(engine.bundle().display_name(Layout::English), "English");
        assert_eq!(engine.bundle().display_name(Layout::Secondary), "Ukrainian");
    }

    #[test]
    fn engine_config_rejects_invalid_runtime_values() {
        assert_eq!(
            EngineConfig {
                min_token_len: 0,
                ..EngineConfig::default()
            }
            .validate(),
            Err(EngineConfigError::MinTokenLenZero)
        );
        assert_eq!(
            EngineConfig {
                min_token_len: 8,
                max_token_len: 4,
                ..EngineConfig::default()
            }
            .validate(),
            Err(EngineConfigError::MinTokenLenGreaterThanMaxTokenLen { min: 8, max: 4 })
        );
        assert_eq!(
            EngineConfig {
                max_token_len: MAX_CONFIG_TOKEN_LEN + 1,
                ..EngineConfig::default()
            }
            .validate(),
            Err(EngineConfigError::MaxTokenLenTooLarge {
                value: MAX_CONFIG_TOKEN_LEN + 1,
                max: MAX_CONFIG_TOKEN_LEN,
            })
        );
        assert_eq!(
            EngineConfig {
                confidence_margin: f32::NAN,
                ..EngineConfig::default()
            }
            .validate(),
            Err(EngineConfigError::InvalidFloat {
                field: "confidence_margin",
            })
        );
    }

    #[test]
    fn it_tracks_token_candidates_from_letter_events() {
        let mut engine = engine();
        for event in letters(&[
            PhysicalKey::G,
            PhysicalKey::H,
            PhysicalKey::S,
            PhysicalKey::D,
            PhysicalKey::B,
            PhysicalKey::N,
        ]) {
            engine.process(InputEvent::Letter(event));
        }

        let candidates = engine.token_candidates();
        assert_eq!(candidates.english, "ghsdbn");
        assert_eq!(candidates.secondary, "привіт");
    }

    #[test]
    fn composition_renders_letters_without_committing_until_boundary() {
        let mut engine = engine();
        let mut committed = String::new();
        let mut composing = String::new();

        for character in "ghsdbn".chars() {
            let input = input_event_for_char(&engine, character);
            let output = engine.process(input);
            apply_composition(
                output.action,
                Some(character),
                &mut committed,
                &mut composing,
            );
        }

        assert_eq!(committed, "");
        assert_eq!(composing, "привіт");
        assert_eq!(engine.current_layout(), Layout::Secondary);

        let output = engine.process(InputEvent::EndToken);
        apply_composition(output.action, None, &mut committed, &mut composing);

        assert_eq!(committed, "привіт");
        assert_eq!(composing, "");
        assert_eq!(engine.token_len(), 0);
    }

    #[test]
    fn literal_commits_active_composition_and_literal_together() {
        let mut engine = engine();
        for character in "abc".chars() {
            let input = input_event_for_char(&engine, character);
            engine.process(input);
        }

        let output = engine.process(InputEvent::Literal('1'));
        assert_eq!(
            output.action,
            CompositionAction::Commit {
                text: "abc1".to_owned(),
                consume_event: true,
            }
        );
        assert_eq!(engine.token_len(), 0);
    }

    #[test]
    fn english_punctuation_key_commits_decided_english_token() {
        let mut engine = engine();
        let rendered = run_text_committed(&mut engine, "hello,ghsdbn");

        assert_eq!(rendered, "hello,привіт");
        assert_eq!(engine.current_layout(), Layout::Secondary);
    }

    #[test]
    fn punctuation_position_keys_can_still_form_secondary_words() {
        let mut engine = engine();
        let rendered = run_text_committed(&mut engine, ",f,f");

        assert_eq!(rendered, "баба");
        assert_eq!(engine.current_layout(), Layout::Secondary);
    }

    #[test]
    fn backspace_updates_owned_composition() {
        let mut engine = engine();
        for character in "type".chars() {
            let input = input_event_for_char(&engine, character);
            engine.process(input);
        }

        let output = engine.process(InputEvent::Backspace);
        assert_eq!(
            output.action,
            CompositionAction::Render {
                text: "typ".to_owned(),
                layout: Layout::English,
            }
        );
        assert_eq!(engine.token_candidates().english, "typ");

        engine.process(InputEvent::Backspace);
        engine.process(InputEvent::Backspace);
        let output = engine.process(InputEvent::Backspace);
        assert_eq!(
            output.action,
            CompositionAction::Clear {
                consume_event: true,
            }
        );
        assert_eq!(engine.token_len(), 0);
    }

    #[test]
    fn manual_switch_changes_composition_not_document_state() {
        let mut engine = engine();
        for character in "type".chars() {
            let input = input_event_for_char(&engine, character);
            engine.process(input);
        }

        let output = engine.force_switch_token();
        assert_eq!(
            output.action,
            CompositionAction::Render {
                text: "ензу".to_owned(),
                layout: Layout::Secondary,
            }
        );
        assert_eq!(engine.current_layout(), Layout::Secondary);
        assert_eq!(engine.token_len(), 4);
    }

    #[test]
    fn internal_caps_and_acronyms_render_without_switching() {
        let mut case_engine = engine();
        for (idx, key) in [
            PhysicalKey::G,
            PhysicalKey::H,
            PhysicalKey::S,
            PhysicalKey::D,
            PhysicalKey::B,
            PhysicalKey::N,
        ]
        .iter()
        .copied()
        .enumerate()
        {
            case_engine.process(InputEvent::Letter(LetterEvent {
                physical_key: key,
                shift: idx % 2 == 1,
            }));
        }

        assert_eq!(case_engine.current_layout(), Layout::English);
        assert_eq!(run_text_committed(&mut case_engine, ""), "gHsDbN");

        let mut engine = engine();
        for key in [
            PhysicalKey::H,
            PhysicalKey::T,
            PhysicalKey::T,
            PhysicalKey::P,
        ] {
            engine.process(InputEvent::Letter(LetterEvent {
                physical_key: key,
                shift: true,
            }));
        }
        assert_eq!(engine.current_layout(), Layout::English);
    }

    #[test]
    fn host_context_bypasses_secure_and_full_disabled_inputs() {
        let mut engine = engine();
        for event in letters(&[PhysicalKey::G, PhysicalKey::H, PhysicalKey::B]) {
            engine.process(InputEvent::Letter(event));
        }
        assert_eq!(engine.token_len(), 3);

        engine.set_host_context(HostContext {
            secure_input: true,
            automatic_processing_disabled: false,
            automatic_switching_disabled: false,
        });
        let output = engine.process(InputEvent::Letter(LetterEvent::new(PhysicalKey::D)));

        assert_eq!(output.decision, Decision::Bypass);
        assert_eq!(output.action, CompositionAction::Bypass);
        assert_eq!(engine.token_len(), 0);

        engine.set_host_context(HostContext {
            secure_input: false,
            automatic_processing_disabled: true,
            automatic_switching_disabled: false,
        });
        let output = engine.process(InputEvent::Letter(LetterEvent::new(PhysicalKey::D)));
        assert_eq!(output.action, CompositionAction::Bypass);
    }

    #[test]
    fn auto_switching_disabled_still_owns_composition_in_current_layout() {
        let mut engine = engine();
        engine.set_host_context(HostContext {
            secure_input: false,
            automatic_processing_disabled: false,
            automatic_switching_disabled: true,
        });

        let rendered = run_text_committed(&mut engine, "ghsdbn");

        assert_eq!(rendered, "ghsdbn");
        assert_eq!(engine.current_layout(), Layout::English);

        engine.reset_layout(Layout::Secondary);
        let rendered = run_text_committed(&mut engine, "ghsdbn");

        assert_eq!(rendered, "привіт");
        assert_eq!(engine.current_layout(), Layout::Secondary);
    }

    #[test]
    fn long_tokens_commit_once_then_bypass_until_boundary() {
        let config = EngineConfig {
            max_token_len: 3,
            ..EngineConfig::default()
        };
        let mut engine = engine_with_config(config);

        for event in letters(&[PhysicalKey::A, PhysicalKey::B, PhysicalKey::C]) {
            let output = engine.process(InputEvent::Letter(event));
            assert!(matches!(output.action, CompositionAction::Render { .. }));
        }
        assert_eq!(engine.token_len(), 3);

        let output = engine.process(InputEvent::Letter(LetterEvent::new(PhysicalKey::D)));
        assert_eq!(
            output.action,
            CompositionAction::Commit {
                text: "abcd".to_owned(),
                consume_event: true,
            }
        );
        assert_eq!(engine.token_len(), 0);

        let output = engine.process(InputEvent::Letter(LetterEvent::new(PhysicalKey::E)));
        assert_eq!(output.action, CompositionAction::Bypass);

        engine.process(InputEvent::EndToken);
        let output = engine.process(InputEvent::Letter(LetterEvent::new(PhysicalKey::F)));
        assert_eq!(
            output.action,
            CompositionAction::Render {
                text: "f".to_owned(),
                layout: Layout::English,
            }
        );
    }

    #[test]
    fn language_bundle_reverse_maps_secondary_characters() {
        let engine = engine();
        let event = engine.letter_event_from_char('ж').unwrap();

        assert_eq!(event.physical_key, PhysicalKey::Semicolon);
        assert_eq!(engine.bundle().render(event, Layout::Secondary), 'ж');
    }

    #[test]
    fn keyboard_map_loads_named_and_custom_layouts() {
        let named = KeyboardMap::named("ukrainian-jcuken-osx").unwrap();
        assert_eq!(named.render(PhysicalKey::Backslash, false), 'ґ');

        let custom = KeyboardMap::from_rows(
            "abcdefghijklmnopqrstuvwxyz`[];',.\\",
            "ABCDEFGHIJKLMNOPQRSTUVWXYZ~{}:\"<>|",
        )
        .unwrap();
        assert_eq!(custom.render(PhysicalKey::A, false), 'a');
        assert_eq!(custom.render(PhysicalKey::Period, true), '>');

        let unsupported = KeyboardMap::from_rows(
            "😀bcdefghijklmnopqrstuvwxyz`[];',.\\",
            "ABCDEFGHIJKLMNOPQRSTUVWXYZ~{}:\"<>|",
        )
        .unwrap_err();
        assert!(unsupported.to_string().contains("not supported"));
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        #[test]
        fn generated_events_preserve_engine_state_invariants(
            events in prop::collection::vec(input_event_strategy(), 0..512)
        ) {
            let mut engine = engine();
            let max_token_len = engine.config().max_token_len;

            for event in events {
                let action = {
                    let output = engine.process(event);
                    output.action.clone()
                };
                let token_len = engine.token_len();
                let candidates = engine.token_candidates();

                prop_assert!(token_len <= max_token_len);
                prop_assert_eq!(candidates.english.chars().count(), token_len);
                prop_assert_eq!(candidates.secondary.chars().count(), token_len);

                match &action {
                    CompositionAction::Render { text, .. } => {
                        prop_assert!(text.chars().count() <= max_token_len);
                    }
                    CompositionAction::Commit { text, .. } => {
                        prop_assert!(text.chars().count() <= max_token_len + 1);
                    }
                    CompositionAction::Bypass | CompositionAction::Clear { .. } => {}
                }

                assert_finite_score(engine.token_score());
            }
        }
    }

    fn letters(physical_keys: &[PhysicalKey]) -> Vec<LetterEvent> {
        physical_keys
            .iter()
            .copied()
            .map(LetterEvent::new)
            .collect()
    }

    fn run_text_committed(engine: &mut Engine, text: &str) -> String {
        let mut committed = String::new();
        let mut composing = String::new();
        for character in text.chars() {
            let input = input_event_for_char(engine, character);
            let output = engine.process(input);
            apply_composition(
                output.action,
                Some(character),
                &mut committed,
                &mut composing,
            );
        }
        let output = engine.process(InputEvent::EndToken);
        apply_composition(output.action, None, &mut committed, &mut composing);
        committed
    }

    fn input_event_for_char(engine: &Engine, character: char) -> InputEvent {
        engine.input_event_from_char(character)
    }

    fn apply_composition(
        action: CompositionAction,
        fallback: Option<char>,
        committed: &mut String,
        composing: &mut String,
    ) {
        match action {
            CompositionAction::Bypass => {
                if let Some(character) = fallback {
                    committed.push(character);
                }
            }
            CompositionAction::Render { text, .. } => {
                *composing = text;
            }
            CompositionAction::Commit {
                text,
                consume_event,
            } => {
                committed.push_str(&text);
                composing.clear();
                if !consume_event && let Some(character) = fallback {
                    committed.push(character);
                }
            }
            CompositionAction::Clear { consume_event } => {
                composing.clear();
                if !consume_event && let Some(character) = fallback {
                    committed.push(character);
                }
            }
        }
    }

    fn assert_finite_score(score: ScoreAnalysis) {
        for value in [
            score.english.total,
            score.english.bigram,
            score.english.trigram,
            score.english.dict_exact_bonus,
            score.english.dict_prefix_bonus,
            score.secondary.total,
            score.secondary.bigram,
            score.secondary.trigram,
            score.secondary.dict_exact_bonus,
            score.secondary.dict_prefix_bonus,
        ] {
            assert!(value.is_finite(), "score component must stay finite");
        }
    }
}
