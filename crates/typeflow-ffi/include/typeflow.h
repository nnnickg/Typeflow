#ifndef TYPEFLOW_H
#define TYPEFLOW_H

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct TfEngine TfEngine;
typedef struct TfHostConfig TfHostConfig;

const char* typeflow_last_error_message(void);

#define TF_EVENT_LETTER     0
#define TF_EVENT_BACKSPACE  1
#define TF_EVENT_END_TOKEN  2
#define TF_EVENT_LITERAL    3

#define TF_MOD_SHIFT    0x01u
#define TF_MOD_CONTROL  0x02u
#define TF_MOD_OPTION   0x04u
#define TF_MOD_COMMAND  0x08u

#define TF_CONTEXT_SECURE_INPUT 0x01u
#define TF_CONTEXT_AUTOMATIC_PROCESSING_DISABLED 0x02u
#define TF_CONTEXT_AUTOMATIC_SWITCHING_DISABLED 0x04u

#define TF_HOST_POLICY_SECURE_INPUT 0x01u
#define TF_HOST_POLICY_AUTOMATIC_PROCESSING_DISABLED 0x02u
#define TF_HOST_POLICY_MANUAL_CONVERSION_DISABLED 0x04u
#define TF_HOST_POLICY_TERMINAL_SURFACE 0x08u

#define TF_HOST_POLICY_REASON_NORMAL 0
#define TF_HOST_POLICY_REASON_SECURE_INPUT 1
#define TF_HOST_POLICY_REASON_TERMINAL_BUNDLE 2
#define TF_HOST_POLICY_REASON_TERMINAL_SURFACE 3
#define TF_HOST_POLICY_REASON_DISABLED_BUNDLE 4
#define TF_HOST_POLICY_REASON_AUTOMATIC_PROCESSING_DISABLED_BUNDLE 5
#define TF_HOST_POLICY_REASON_UNAVAILABLE_HOST_CONFIG 255

typedef struct {
    uint8_t tag;
    uint8_t physical;
    uint8_t modifiers;
    uint32_t codepoint;
} TfEvent;

typedef struct {
    size_t min_token_len;
    size_t max_token_len;
    float  confidence_margin;
    float  dict_exact_weight;
    float  dict_prefix_weight;
    float  ngram_only_confidence_margin;
    float  bigram_weight;
    float  trigram_weight;
    uint8_t length_normalize;
    uint8_t disable_on_internal_caps;
} TfEngineConfig;

#define TF_ACTION_KEEP    0
#define TF_ACTION_COMMIT  1
#define TF_ACTION_REPLACE 2
#define TF_ACTION_RESET   3

#define TF_LAYOUT_ENGLISH 0
#define TF_LAYOUT_SECONDARY 1

#define TF_REPLACE_BUF_LEN 4096

typedef struct {
    uint8_t  tag;
    uint32_t commit_codepoint;
    size_t   replace_old_len;
    size_t   replace_text_len;
    uint8_t  replace_layout;
    uint8_t  replace_text[TF_REPLACE_BUF_LEN];
} TfAction;

typedef struct {
    uint8_t     secure_input;
    const char* bundle_id_utf8;
    const char* application_name_utf8;
    const char* input_client_class_utf8;
    const char* focused_element_role_utf8;
    const char* focused_element_subrole_utf8;
    const char* focused_element_role_description_utf8;
    const char* focused_element_identifier_utf8;
    const char* focused_element_description_utf8;
    const char* focused_window_title_utf8;
} TfHostSurfaceFacts;

typedef struct {
    uint32_t flags;
    uint8_t  reason;
} TfHostInputPolicy;

TfEngine* typeflow_engine_new_embedded(void);
TfEngine* typeflow_engine_new_embedded_with_config(TfEngineConfig config);
TfEngine* typeflow_engine_new_from_data_dir(const char* data_dir_utf8);
TfEngine* typeflow_engine_new_from_data_dir_with_config(const char* data_dir_utf8, TfEngineConfig config);
TfEngine* typeflow_engine_new_from_pack_dir(const char* pack_dir_utf8);
TfEngine* typeflow_engine_new_from_pack_dir_with_config(const char* pack_dir_utf8, TfEngineConfig config);
TfEngine* typeflow_engine_new_from_host_config(const TfHostConfig* config);
void      typeflow_engine_free(TfEngine* engine);
void      typeflow_engine_reset_token(TfEngine* engine);
void      typeflow_engine_reset_layout(TfEngine* engine, uint8_t layout);
void      typeflow_engine_set_host_context(TfEngine* engine, uint32_t flags);
void      typeflow_engine_force_switch_token(TfEngine* engine, TfAction* out_action);
void      typeflow_engine_convert_visible_token(TfEngine* engine, const char* token_utf8, TfAction* out_action);
void      typeflow_engine_convert_visible_tail(TfEngine* engine, const char* visible_tail_utf8, TfAction* out_action);
void      typeflow_engine_replace_visible_prefix_with_key(TfEngine* engine, const char* visible_prefix_utf8, uint8_t physical, uint8_t modifiers, uint8_t target_layout, TfAction* out_action);
void      typeflow_engine_replace_visible_tail_with_key(TfEngine* engine, const char* visible_tail_utf8, uint8_t physical, uint8_t modifiers, uint8_t target_layout, TfAction* out_action);
uint8_t   typeflow_engine_current_layout(TfEngine* engine);
void      typeflow_engine_process(TfEngine* engine, TfEvent event, TfAction* out_action);
void      typeflow_engine_default_config(TfEngineConfig* out_config);

TfHostConfig* typeflow_host_config_load(void);
TfHostConfig* typeflow_host_config_load_defaults(void);
TfHostConfig* typeflow_host_config_load_with_environment(const char* config_path_utf8, const char* home_utf8, const char* data_dir_utf8, const char* pack_dir_utf8);
void          typeflow_host_config_free(TfHostConfig* config);
void          typeflow_host_config_engine_config(const TfHostConfig* config, TfEngineConfig* out_config);
const char*   typeflow_host_config_source_path(const TfHostConfig* config);
const char*   typeflow_host_config_secondary_language(const TfHostConfig* config);
const char*   typeflow_host_config_pack_directory(const TfHostConfig* config);
const char*   typeflow_host_config_data_directory(const TfHostConfig* config);
const char*   typeflow_host_config_engine_source(const TfHostConfig* config);
uint8_t       typeflow_host_config_is_bundle_disabled(const TfHostConfig* config, const char* bundle_id_utf8);
uint8_t       typeflow_host_config_is_automatic_processing_disabled(const TfHostConfig* config, const char* bundle_id_utf8);
size_t        typeflow_host_config_disabled_bundle_count(const TfHostConfig* config);
size_t        typeflow_host_config_auto_disabled_bundle_count(const TfHostConfig* config);
void          typeflow_host_config_resolve_input_policy(const TfHostConfig* config, TfHostSurfaceFacts facts, TfHostInputPolicy* out_policy);

#ifdef __cplusplus
}
#endif

#endif
