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
    Action, Decision, EngineConfig, EngineConfigError, EngineOutput, HostContext, InputEvent,
    Layout, LayoutCandidates, LayoutScore, MAX_CONFIG_TOKEN_LEN, ScoreAnalysis,
};

#[cfg(test)]
mod tests {
    use super::{
        Action, Decision, Engine, EngineConfig, EngineConfigError, HostContext, InputEvent,
        KeyboardMap, Layout, LetterEvent, MAX_CONFIG_TOKEN_LEN, PhysicalKey,
    };
    use crate::data::LanguageBundle;

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
    fn it_scores_ukrainian_higher_for_pryvit() {
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
        let score = engine.score(&engine.token_candidates());
        assert!(
            score.secondary.total > score.english.total,
            "expected ukrainian > english, got {:?} vs {:?}",
            score.secondary,
            score.english
        );
    }

    #[test]
    fn it_scores_english_higher_for_typeflow() {
        let mut engine = engine();
        for event in letters(&[
            PhysicalKey::T,
            PhysicalKey::Y,
            PhysicalKey::P,
            PhysicalKey::E,
            PhysicalKey::F,
            PhysicalKey::L,
            PhysicalKey::O,
            PhysicalKey::W,
        ]) {
            engine.process(InputEvent::Letter(event));
        }
        let score = engine.score(&engine.token_candidates());
        assert!(score.english.total > score.secondary.total);
    }

    #[test]
    fn it_replaces_token_when_decision_switches_layout() {
        let mut engine = engine();
        let mut last_action = Action::Keep;
        for event in letters(&[
            PhysicalKey::G,
            PhysicalKey::H,
            PhysicalKey::S,
            PhysicalKey::D,
            PhysicalKey::B,
            PhysicalKey::N,
        ]) {
            last_action = engine.process(InputEvent::Letter(event)).action;
        }

        // Engine should have flipped to Ukrainian at some point during the token.
        assert_eq!(engine.current_layout(), Layout::Secondary);
        // The final action should either be a Commit (already in Ukrainian) or Replace
        // (just flipped on this letter); both are acceptable depending on calibration.
        assert!(matches!(
            last_action,
            Action::Commit(_) | Action::ReplaceToken { .. }
        ));
    }

    #[test]
    fn it_keeps_layout_for_short_tokens() {
        let mut engine = engine();
        engine.process(InputEvent::Letter(LetterEvent::new(PhysicalKey::G)));
        let output = engine.process(InputEvent::Letter(LetterEvent::new(PhysicalKey::H)));
        assert_eq!(output.decision, Decision::Keep);
        assert_eq!(output.action, Action::Commit('h'));
    }

    #[test]
    fn it_resets_token_on_end_token() {
        let mut engine = engine();
        engine.process(InputEvent::Letter(LetterEvent::new(PhysicalKey::T)));
        let output = engine.process(InputEvent::EndToken);
        assert_eq!(output.action, Action::ResetToken);
        assert_eq!(engine.token_len(), 0);
    }

    #[test]
    fn it_pops_token_on_backspace() {
        let mut engine = engine();
        for event in letters(&[PhysicalKey::T, PhysicalKey::Y, PhysicalKey::P]) {
            engine.process(InputEvent::Letter(event));
        }
        let output = engine.process(InputEvent::Backspace);
        assert_eq!(output.action, Action::Keep);
        assert_eq!(engine.token_len(), 2);
        assert_eq!(engine.token_candidates().english, "ty");
    }

    #[test]
    fn it_reverts_layout_when_backspacing_before_the_switch_point() {
        let mut engine = engine();
        for event in letters(&[
            PhysicalKey::G,
            PhysicalKey::H,
            PhysicalKey::S,
            PhysicalKey::D,
        ]) {
            engine.process(InputEvent::Letter(event));
        }
        assert_eq!(engine.current_layout(), Layout::Secondary);

        engine.process(InputEvent::Backspace);

        assert_eq!(engine.token_candidates().english, "ghs");
        assert_eq!(engine.current_layout(), Layout::English);
    }

    #[test]
    fn backspace_on_empty_token_is_a_noop() {
        let mut engine = engine();
        let output = engine.process(InputEvent::Backspace);
        assert_eq!(output.action, Action::Keep);
        assert_eq!(engine.token_len(), 0);
    }

    #[test]
    fn it_refuses_to_switch_on_internal_caps() {
        let mut engine = engine();
        // Type "gHsDbN" — same physical keys as привіт but with mid-word capitals.
        // Engine should refuse to switch layouts because this looks like an identifier.
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
            engine.process(InputEvent::Letter(LetterEvent {
                physical_key: key,
                // shift on every odd-indexed letter (positions 1, 3, 5)
                shift: idx % 2 == 1,
            }));
        }
        assert_eq!(engine.current_layout(), Layout::English);
    }

    #[test]
    fn it_does_not_block_capitalized_first_letter() {
        let mut engine = engine();
        // Type "Привіт" via physical keys with shift on position 0 only.
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
            engine.process(InputEvent::Letter(LetterEvent {
                physical_key: key,
                shift: idx == 0,
            }));
        }
        assert_eq!(engine.current_layout(), Layout::Secondary);

        let score = engine.score(&engine.token_candidates());
        assert!(score.secondary.exact_count > 0);
    }

    #[test]
    fn it_renders_shifted_english_punctuation_positions() {
        let bundle = fixture_bundle();
        let cases = [
            ('~', PhysicalKey::Grave),
            ('{', PhysicalKey::LBracket),
            ('}', PhysicalKey::RBracket),
            (':', PhysicalKey::Semicolon),
            ('"', PhysicalKey::Quote),
            ('<', PhysicalKey::Comma),
            ('>', PhysicalKey::Period),
            ('|', PhysicalKey::Backslash),
        ];

        for (character, key) in cases {
            let event = LetterEvent::from_char(character).unwrap();

            assert_eq!(event.physical_key, key);
            assert!(event.shift);
            assert_eq!(bundle.render(event, Layout::English), character);
        }
    }

    #[test]
    fn it_requires_stronger_margin_without_dictionary_evidence() {
        let config = EngineConfig {
            confidence_margin: 0.0,
            ngram_only_confidence_margin: f32::MAX,
            ..EngineConfig::default()
        };

        let mut engine = engine_with_config(config);
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
        assert_eq!(engine.current_layout(), Layout::Secondary);
        engine.process(InputEvent::EndToken);

        for event in letters(&[
            PhysicalKey::Q,
            PhysicalKey::W,
            PhysicalKey::E,
            PhysicalKey::R,
            PhysicalKey::T,
            PhysicalKey::Y,
        ]) {
            engine.process(InputEvent::Letter(event));
        }

        assert_eq!(engine.current_layout(), Layout::Secondary);
    }

    #[test]
    fn literal_resets_token_and_commits_the_character() {
        let mut engine = engine();
        for event in letters(&[PhysicalKey::G, PhysicalKey::H, PhysicalKey::B]) {
            engine.process(InputEvent::Letter(event));
        }
        assert_eq!(engine.token_len(), 3);

        let output = engine.process(InputEvent::Literal('1'));
        assert_eq!(output.action, Action::Commit('1'));
        assert_eq!(engine.token_len(), 0);

        // The remaining letters form a new short token; engine stays English
        // because each segment is below min_token_len.
        for event in letters(&[PhysicalKey::D, PhysicalKey::T, PhysicalKey::N]) {
            engine.process(InputEvent::Letter(event));
        }
        assert_eq!(engine.current_layout(), Layout::English);
    }

    #[test]
    fn english_punctuation_key_ends_decided_english_token() {
        let mut engine = engine();
        let rendered = run_cli_like_token(&mut engine, "hello,ghsdbn");

        assert_eq!(rendered, "hello,привіт");
        assert_eq!(engine.current_layout(), Layout::Secondary);
    }

    #[test]
    fn punctuation_position_keys_can_still_form_secondary_words() {
        let mut engine = engine();
        let rendered = run_cli_like_token(&mut engine, ",f,f");

        assert_eq!(rendered, "баба");
        assert_eq!(engine.current_layout(), Layout::Secondary);
    }

    #[test]
    fn backspace_after_literal_keeps_token_state_consistent() {
        // Reproduces the desync that existed when literals didn't terminate the
        // token: the host buffer would drift from `engine.token` because each
        // literal added a character to the host while leaving `engine.token`
        // unchanged. With literal = reset+commit, both sides advance in lockstep.
        let mut engine = engine();
        let mut committed = String::new();

        for character in "abc".chars() {
            let action = engine.process_action(input_event_for_char(&engine, character));
            apply_action_to_string(&action, &mut committed);
        }
        let action = engine.process_action(InputEvent::Literal('1'));
        apply_action_to_string(&action, &mut committed);
        for character in "def".chars() {
            let action = engine.process_action(input_event_for_char(&engine, character));
            apply_action_to_string(&action, &mut committed);
        }

        assert_eq!(committed, "abc1def");
        assert_eq!(engine.token_len(), 3); // engine sees only "def"

        // Backspace 3 times via the engine; the host pops the corresponding
        // char itself (Backspace returns Action::Keep).
        for _ in 0..3 {
            engine.process_action(InputEvent::Backspace);
            committed.pop();
        }
        assert_eq!(engine.token_len(), 0);
        assert_eq!(committed, "abc1");
    }

    #[test]
    fn it_bypasses_acronym_like_tokens() {
        let mut engine = engine();
        let mut output = engine.process(InputEvent::Letter(LetterEvent {
            physical_key: PhysicalKey::H,
            shift: true,
        }));
        for key in [PhysicalKey::T, PhysicalKey::T, PhysicalKey::P] {
            output = engine.process(InputEvent::Letter(LetterEvent {
                physical_key: key,
                shift: true,
            }));
        }

        assert_eq!(output.decision, Decision::Bypass);
        assert_eq!(engine.current_layout(), Layout::English);
    }

    #[test]
    fn it_keeps_devops_and_secret_like_tokens_english() {
        let cases = [
            "http",
            "https://example.com",
            "user@example.com",
            "/var/log/nginx/access.log",
            "snake_case",
            "camelCase",
            "HTTP",
            "abc123",
            "CLOUDACCESSKEYIDLIKEVALUE1234",
            "arn:aws:iam::000000000000:role/ExampleRole",
            "kubectl",
            "terraform",
            "Bearer",
            "github-token-placeholder",
            "password123!",
            "kube-system",
            "deployment.apps",
        ];

        for token in cases {
            let mut engine = engine();
            let rendered = run_cli_like_token(&mut engine, token);

            assert_eq!(
                engine.current_layout(),
                Layout::English,
                "false positive for {token}, rendered {rendered}"
            );
            assert_eq!(rendered, token);
        }
    }

    #[test]
    fn weird_unicode_literals_do_not_panic_or_switch() {
        let mut engine = engine();
        let mut committed = String::new();

        for character in ['🧪', '\u{200d}', '\u{0301}', '\n', '\u{0000}', 'ß'] {
            let output = engine.process(InputEvent::Literal(character));
            apply_action_to_string(&output.action, &mut committed);
        }

        assert_eq!(engine.current_layout(), Layout::English);
        assert_eq!(engine.token_len(), 0);
    }

    #[test]
    fn host_context_bypasses_secure_and_excluded_inputs() {
        let mut engine = engine();
        for event in letters(&[PhysicalKey::G, PhysicalKey::H, PhysicalKey::B]) {
            engine.process(InputEvent::Letter(event));
        }
        assert_eq!(engine.token_len(), 3);

        engine.set_host_context(HostContext {
            secure_input: true,
            app_excluded: false,
        });
        let output = engine.process(InputEvent::Letter(LetterEvent::new(PhysicalKey::D)));

        assert_eq!(output.decision, Decision::Bypass);
        assert_eq!(output.action, Action::Keep);
        assert_eq!(engine.token_len(), 0);

        engine.set_host_context(HostContext {
            secure_input: false,
            app_excluded: true,
        });
        let output = engine.process(InputEvent::Letter(LetterEvent::new(PhysicalKey::D)));
        assert_eq!(output.action, Action::Keep);
    }

    #[test]
    fn modifier_bypass_event_does_not_commit_or_score() {
        let mut engine = engine();
        let output = engine.process(InputEvent::HostBypass);

        assert_eq!(output.decision, Decision::Bypass);
        assert_eq!(output.action, Action::Keep);
        assert_eq!(engine.token_len(), 0);
    }

    #[test]
    fn long_tokens_bypass_until_boundary() {
        let config = EngineConfig {
            max_token_len: 3,
            ..EngineConfig::default()
        };
        let mut engine = engine_with_config(config);

        for event in letters(&[PhysicalKey::A, PhysicalKey::B, PhysicalKey::C]) {
            let output = engine.process(InputEvent::Letter(event));
            assert_eq!(output.decision, Decision::Keep);
        }
        assert_eq!(engine.token_len(), 3);

        let output = engine.process(InputEvent::Letter(LetterEvent::new(PhysicalKey::D)));
        assert_eq!(output.decision, Decision::Bypass);
        assert_eq!(output.action, Action::Commit('d'));
        assert_eq!(engine.token_len(), 0);

        let output = engine.process(InputEvent::Letter(LetterEvent::new(PhysicalKey::E)));
        assert_eq!(output.decision, Decision::Bypass);
        assert_eq!(output.action, Action::Commit('e'));
        assert_eq!(engine.token_len(), 0);

        engine.process(InputEvent::EndToken);
        let output = engine.process(InputEvent::Letter(LetterEvent::new(PhysicalKey::F)));
        assert_eq!(output.decision, Decision::Keep);
        assert_eq!(output.action, Action::Commit('f'));
        assert_eq!(engine.token_len(), 1);
    }

    #[test]
    fn action_only_path_matches_full_output_for_switching_token() {
        let mut full = engine();
        let mut fast = engine();
        let mut full_committed = String::new();
        let mut fast_committed = String::new();

        for character in "ghsdbn".chars() {
            let input = input_event_for_char(&full, character);
            let output = full.process(input);
            apply_action_to_string(&output.action, &mut full_committed);

            let input = input_event_for_char(&fast, character);
            let action = fast.process_action(input);
            apply_action_to_string(&action, &mut fast_committed);
        }

        assert_eq!(full_committed, "привіт");
        assert_eq!(fast_committed, full_committed);
        assert_eq!(fast.current_layout(), full.current_layout());
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
    }

    #[test]
    fn replace_token_old_len_matches_committed_prefix() {
        // When the engine flips mid-stream, the host has only committed the
        // previous letters of the token (one Commit per letter). `old_len` must
        // equal that committed-prefix length so AppKit's
        // `client.insertText(_:replacementRange:)` can target a real range
        // instead of going off the end of the buffer.
        let mut engine = engine();
        let mut committed = String::new();
        let mut flip_action: Option<Action> = None;

        for character in "ghsdbn".chars() {
            let action = engine.process_action(input_event_for_char(&engine, character));
            if matches!(action, Action::ReplaceToken { .. }) {
                flip_action = Some(action.clone());
                let committed_before_flip = committed.chars().count();
                if let Action::ReplaceToken { old_len, .. } = &action {
                    assert_eq!(
                        *old_len, committed_before_flip,
                        "old_len ({old_len}) must equal the host's committed prefix \
                         ({committed_before_flip}) at the moment of the flip"
                    );
                }
            }
            apply_action_to_string(&action, &mut committed);
        }

        assert_eq!(committed, "привіт");
        assert!(
            flip_action.is_some(),
            "expected a ReplaceToken event for ghsdbn -> привіт"
        );
    }

    #[test]
    fn force_switch_old_len_matches_committed_token() {
        // For force_switch, every letter of the token has already been
        // committed; old_len must equal the full token length (no off-by-one).
        let mut engine = engine();
        let mut committed = String::new();
        for character in "type".chars() {
            let action = engine.process_action(input_event_for_char(&engine, character));
            apply_action_to_string(&action, &mut committed);
        }
        assert_eq!(committed.chars().count(), 4);

        let output = engine.force_switch_token();
        let Action::ReplaceToken { old_len, .. } = output.action else {
            panic!("expected ReplaceToken from force_switch");
        };
        assert_eq!(old_len, 4);
    }

    #[test]
    fn it_force_switches_the_current_token() {
        let mut engine = engine();
        for event in letters(&[
            PhysicalKey::T,
            PhysicalKey::Y,
            PhysicalKey::P,
            PhysicalKey::E,
        ]) {
            engine.process(InputEvent::Letter(event));
        }

        let output = engine.force_switch_token();

        assert_eq!(engine.current_layout(), Layout::Secondary);
        assert_eq!(
            output.action,
            Action::ReplaceToken {
                old_len: 4,
                replacement: "ензу".to_owned(),
                layout: Layout::Secondary,
            }
        );
    }

    #[test]
    fn visible_token_conversion_uses_actual_text() {
        let engine = engine();

        assert_eq!(
            engine.convert_visible_token("afrn"),
            Some((Layout::Secondary, "факт".to_owned()))
        );
        assert_eq!(
            engine.convert_visible_token("факт"),
            Some((Layout::English, "afrn".to_owned()))
        );
        assert_eq!(
            engine.convert_visible_token("ghив"),
            Some((Layout::Secondary, "прив".to_owned()))
        );
    }

    #[test]
    fn visible_prefix_replacement_uses_host_prefix_length() {
        let mut engine = engine();
        let action = engine
            .replace_visible_prefix_with_key(
                "ghb",
                LetterEvent::new(PhysicalKey::D),
                Layout::Secondary,
            )
            .unwrap();

        assert_eq!(
            action,
            Action::ReplaceToken {
                old_len: 3,
                replacement: "прів".to_owned(),
                layout: Layout::Secondary,
            }
        );
        assert_eq!(engine.current_layout(), Layout::Secondary);
    }

    #[test]
    fn visible_tail_keeps_punctuation_position_letters_in_token() {
        let mut engine = engine();

        assert_eq!(engine.visible_token_suffix("hello [eqy"), Some("[eqy"));
        assert_eq!(
            engine.convert_visible_tail("hello [eqyz"),
            Some((Layout::Secondary, "хуйня".to_owned(), 5))
        );

        let action = engine
            .replace_visible_tail_with_key(
                "hello [eqy",
                LetterEvent::new(PhysicalKey::Z),
                Layout::Secondary,
            )
            .unwrap();

        assert_eq!(
            action,
            Action::ReplaceToken {
                old_len: 4,
                replacement: "хуйня".to_owned(),
                layout: Layout::Secondary,
            }
        );
    }

    fn letters(physical_keys: &[PhysicalKey]) -> Vec<LetterEvent> {
        physical_keys
            .iter()
            .copied()
            .map(LetterEvent::new)
            .collect()
    }

    fn run_cli_like_token(engine: &mut Engine, token: &str) -> String {
        let mut committed = String::new();
        for character in token.chars() {
            let input = input_event_for_char(engine, character);
            let action = engine.process_action(input);
            apply_action_to_string(&action, &mut committed);
        }
        committed
    }

    fn input_event_for_char(engine: &Engine, character: char) -> InputEvent {
        engine.input_event_from_char(character)
    }

    fn apply_action_to_string(action: &Action, committed: &mut String) {
        match action {
            Action::Keep | Action::ResetToken => {}
            Action::Commit(character) => committed.push(*character),
            Action::ReplaceToken {
                old_len,
                replacement,
                ..
            } => {
                for _ in 0..*old_len {
                    committed.pop();
                }
                committed.push_str(replacement);
            }
        }
    }
}
