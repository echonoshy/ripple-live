#include "passport_realtime.h"

#include "passport_policy.h"
#include "passport_ui.h"
#include "bsp_audio.h"
#include "cJSON.h"
#include "esp_log.h"
#include "esp_timer.h"
#include "esp_websocket_client.h"
#include "freertos/FreeRTOS.h"
#include "freertos/event_groups.h"
#include "freertos/queue.h"
#include "freertos/semphr.h"
#include "freertos/task.h"
#include "mbedtls/base64.h"

#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define SESSION_READY BIT0
#define RECORDING BIT1
#define RESPONSE_DONE BIT2
#define PTT_RELEASED BIT3

#define PLAYBACK_QUEUE_CHUNKS 8
#define OUTBOUND_QUEUE_ITEMS 8
#define MAX_WS_MESSAGE_BYTES 16384U
#define MAX_OUTPUT_BASE64_BYTES (4U * ((PASSPORT_OUTPUT_MAX_SAMPLES * sizeof(float) + 2U) / 3U))
#define SEND_TIMEOUT_MS 750U
#define SESSION_READY_TIMEOUT_US (15LL * 1000LL * 1000LL)
#define RESPONSE_START_TIMEOUT_US (60LL * 1000LL * 1000LL)
#define RESPONSE_IDLE_TIMEOUT_US (30LL * 1000LL * 1000LL)

typedef enum {
    CONTROL_PRESS = 1,
} control_event_t;

typedef enum {
    OUTBOUND_SESSION_START = 1,
    OUTBOUND_CANCEL_INPUT,
    OUTBOUND_SPEECH_START,
    OUTBOUND_AUDIO,
    OUTBOUND_COMMIT,
    OUTBOUND_ABORT,
} outbound_type_t;

typedef struct {
    outbound_type_t type;
    uint32_t generation;
    char turn_id[48];
    int16_t pcm[PASSPORT_INPUT_SAMPLES];
} outbound_item_t;

typedef struct {
    int16_t *pcm;
    size_t bytes;
    size_t samples;
    uint32_t generation;
} audio_chunk_t;

typedef struct {
    bool accepting;
    bool playback_active;
    bool waiting_response;
    bool turn_valid;
    uint32_t response_generation;
    uint32_t turn_generation;
    size_t queued_samples;
    int64_t session_deadline_us;
    int64_t response_deadline_us;
    char response_id[PASSPORT_RESPONSE_ID_SIZE];
} state_snapshot_t;

static const char *TAG = "passport_ws";
static esp_websocket_client_handle_t s_client;
static EventGroupHandle_t s_flags;
static QueueHandle_t s_controls;
static QueueHandle_t s_outbound;
static QueueHandle_t s_audio;
static SemaphoreHandle_t s_audio_lock;
static TaskHandle_t s_recording_task;
static TaskHandle_t s_network_task;
static TaskHandle_t s_playback_task;
static TaskHandle_t s_supervisor_task;
static portMUX_TYPE s_state_mux = portMUX_INITIALIZER_UNLOCKED;
static volatile uint8_t s_volume = 70;
static bool s_accept_audio;
static bool s_playback_active;
static bool s_waiting_response;
static bool s_turn_valid;
static uint32_t s_response_generation;
static uint32_t s_turn_generation;
static size_t s_queued_samples;
static int64_t s_session_deadline_us;
static int64_t s_response_deadline_us;
static char s_active_response_id[PASSPORT_RESPONSE_ID_SIZE];
static char *s_message;
static size_t s_message_capacity;
static size_t s_message_received;
static size_t s_frame_base;
static size_t s_message_size;
static bool s_fragment_active;
static bool s_discard_message;

static bool websocket_ready(void)
{
    return s_client && s_flags && esp_websocket_client_is_connected(s_client) &&
           (xEventGroupGetBits(s_flags) & SESSION_READY);
}

static state_snapshot_t state_snapshot(void)
{
    state_snapshot_t snapshot;
    portENTER_CRITICAL(&s_state_mux);
    snapshot.accepting = s_accept_audio;
    snapshot.playback_active = s_playback_active;
    snapshot.waiting_response = s_waiting_response;
    snapshot.turn_valid = s_turn_valid;
    snapshot.response_generation = s_response_generation;
    snapshot.turn_generation = s_turn_generation;
    snapshot.queued_samples = s_queued_samples;
    snapshot.session_deadline_us = s_session_deadline_us;
    snapshot.response_deadline_us = s_response_deadline_us;
    memcpy(snapshot.response_id, s_active_response_id, sizeof(snapshot.response_id));
    portEXIT_CRITICAL(&s_state_mux);
    return snapshot;
}

static void clear_audio_queue(void)
{
    if (!s_audio) return;
    audio_chunk_t chunk;
    while (xQueueReceive(s_audio, &chunk, 0) == pdTRUE) free(chunk.pcm);
    portENTER_CRITICAL(&s_state_mux);
    s_queued_samples = 0;
    portEXIT_CRITICAL(&s_state_mux);
}

static void invalidate_response(void)
{
    portENTER_CRITICAL(&s_state_mux);
    s_accept_audio = false;
    s_playback_active = false;
    s_waiting_response = false;
    s_response_deadline_us = 0;
    s_active_response_id[0] = '\0';
    s_response_generation++;
    s_queued_samples = 0;
    portEXIT_CRITICAL(&s_state_mux);
    if (s_flags) xEventGroupClearBits(s_flags, RESPONSE_DONE);
    clear_audio_queue();
}

static void invalidate_turn(void)
{
    portENTER_CRITICAL(&s_state_mux);
    s_turn_valid = false;
    s_turn_generation++;
    portEXIT_CRITICAL(&s_state_mux);
}

static uint32_t begin_turn(void)
{
    portENTER_CRITICAL(&s_state_mux);
    s_turn_generation++;
    s_turn_valid = true;
    uint32_t generation = s_turn_generation;
    portEXIT_CRITICAL(&s_state_mux);
    return generation;
}

static bool turn_is_valid(uint32_t generation)
{
    portENTER_CRITICAL(&s_state_mux);
    bool valid = s_turn_valid && s_turn_generation == generation;
    portEXIT_CRITICAL(&s_state_mux);
    return valid;
}

static void fail_turn(uint32_t generation, const char *detail)
{
    portENTER_CRITICAL(&s_state_mux);
    if (s_turn_generation == generation) s_turn_valid = false;
    portEXIT_CRITICAL(&s_state_mux);
    passport_ui_set(PASSPORT_UI_ERROR, detail);
}

static void expect_response(uint32_t generation)
{
    portENTER_CRITICAL(&s_state_mux);
    if (s_turn_generation == generation && s_turn_valid) {
        s_waiting_response = true;
        s_response_deadline_us = esp_timer_get_time() + RESPONSE_START_TIMEOUT_US;
    }
    portEXIT_CRITICAL(&s_state_mux);
}

static void finish_turn_send(uint32_t generation, bool success)
{
    portENTER_CRITICAL(&s_state_mux);
    if (s_turn_generation == generation) {
        s_turn_valid = false;
        if (!success) {
            s_waiting_response = false;
            s_response_deadline_us = 0;
        }
    }
    portEXIT_CRITICAL(&s_state_mux);
}

static bool response_matches(const char *response_id)
{
    char active[PASSPORT_RESPONSE_ID_SIZE];
    portENTER_CRITICAL(&s_state_mux);
    memcpy(active, s_active_response_id, sizeof(active));
    portEXIT_CRITICAL(&s_state_mux);
    return passport_response_id_matches(active, response_id);
}

static bool begin_response(const char *response_id)
{
    if (!passport_response_id_is_valid(response_id) ||
        (xEventGroupGetBits(s_flags) & RECORDING)) {
        return false;
    }

    portENTER_CRITICAL(&s_state_mux);
    bool expected = s_waiting_response;
    if (expected) {
        s_response_generation++;
        s_accept_audio = true;
        s_playback_active = true;
        s_waiting_response = false;
        s_queued_samples = 0;
        s_response_deadline_us = esp_timer_get_time() + RESPONSE_IDLE_TIMEOUT_US;
        snprintf(s_active_response_id, sizeof(s_active_response_id), "%s", response_id);
    }
    portEXIT_CRITICAL(&s_state_mux);
    if (expected) {
        xEventGroupClearBits(s_flags, RESPONSE_DONE);
        clear_audio_queue();
    }
    return expected;
}

static void mark_response_done(void)
{
    portENTER_CRITICAL(&s_state_mux);
    s_accept_audio = false;
    s_waiting_response = false;
    s_response_deadline_us = 0;
    portEXIT_CRITICAL(&s_state_mux);
    xEventGroupSetBits(s_flags, RESPONSE_DONE);
}

static void complete_playback(uint32_t generation)
{
    portENTER_CRITICAL(&s_state_mux);
    bool current = s_playback_active && s_response_generation == generation;
    if (current) {
        s_accept_audio = false;
        s_playback_active = false;
        s_active_response_id[0] = '\0';
        s_response_generation++;
        s_queued_samples = 0;
    }
    portEXIT_CRITICAL(&s_state_mux);
    if (!current) return;
    xEventGroupClearBits(s_flags, RESPONSE_DONE);
    clear_audio_queue();
    if (websocket_ready() && !(xEventGroupGetBits(s_flags) & RECORDING)) {
        passport_ui_set(PASSPORT_UI_READY, "Connected");
    }
}

static bool queue_outbound(outbound_type_t type, uint32_t generation, const char *turn_id,
                           const int16_t *pcm, TickType_t timeout, bool front)
{
    outbound_item_t item = {.type = type, .generation = generation};
    if (turn_id) snprintf(item.turn_id, sizeof(item.turn_id), "%s", turn_id);
    if (pcm) memcpy(item.pcm, pcm, sizeof(item.pcm));
    BaseType_t result = front ? xQueueSendToFront(s_outbound, &item, timeout)
                              : xQueueSend(s_outbound, &item, timeout);
    return result == pdTRUE;
}

static int send_text(const char *text)
{
    if (!s_client || !esp_websocket_client_is_connected(s_client)) return -1;
    return esp_websocket_client_send_text(s_client, text, strlen(text),
                                          pdMS_TO_TICKS(SEND_TIMEOUT_MS));
}

static bool send_input_audio(const int16_t *pcm)
{
    float *samples = malloc(PASSPORT_INPUT_SAMPLES * sizeof(float));
    if (!samples) return false;
    for (size_t i = 0; i < PASSPORT_INPUT_SAMPLES; ++i) samples[i] = pcm[i] / 32768.0f;

    size_t encoded_size = 4 * ((PASSPORT_INPUT_SAMPLES * sizeof(float) + 2) / 3) + 1;
    unsigned char *encoded = malloc(encoded_size);
    if (!encoded) {
        free(samples);
        return false;
    }
    size_t encoded_len = 0;
    int base64_result = mbedtls_base64_encode(encoded, encoded_size, &encoded_len,
                                               (const unsigned char *)samples,
                                               PASSPORT_INPUT_SAMPLES * sizeof(float));
    free(samples);
    if (base64_result != 0) {
        free(encoded);
        return false;
    }

    size_t event_size = encoded_len + 96;
    char *event = malloc(event_size);
    if (!event) {
        free(encoded);
        return false;
    }
    int length = snprintf(event, event_size,
                          "{\"type\":\"input.audio.append\",\"sample_rate\":16000,\"audio\":\"%.*s\"}",
                          (int)encoded_len, encoded);
    free(encoded);
    bool sent = length > 0 && (size_t)length < event_size && send_text(event) == length;
    free(event);
    return sent;
}

static void network_task(void *arg)
{
    (void)arg;
    outbound_item_t item;
    for (;;) {
        if (xQueueReceive(s_outbound, &item, portMAX_DELAY) != pdTRUE) continue;

        if (item.type == OUTBOUND_SESSION_START) {
            if (send_text("{\"type\":\"session.start\",\"protocol_version\":5,"
                          "\"client_build\":\"passport-0.2\",\"mode\":\"audio\"}") < 0) {
                ESP_LOGW(TAG, "session.start send failed");
            }
            continue;
        }
        if (item.type == OUTBOUND_ABORT) {
            send_text("{\"type\":\"response.cancel\",\"clear_input\":true}");
            continue;
        }
        if (!turn_is_valid(item.generation)) continue;

        bool sent = false;
        if (item.type == OUTBOUND_CANCEL_INPUT) {
            sent = send_text("{\"type\":\"response.cancel\",\"clear_input\":true}") >= 0;
        } else if (item.type == OUTBOUND_SPEECH_START) {
            char event[128];
            int length = snprintf(event, sizeof(event),
                                  "{\"type\":\"input.speech_started\",\"turn_id\":\"%s\"}",
                                  item.turn_id);
            sent = length > 0 && (size_t)length < sizeof(event) && send_text(event) == length;
        } else if (item.type == OUTBOUND_AUDIO) {
            sent = send_input_audio(item.pcm);
        } else if (item.type == OUTBOUND_COMMIT) {
            char event[112];
            int length = snprintf(event, sizeof(event),
                                  "{\"type\":\"input.commit\",\"turn_id\":\"%s\"}",
                                  item.turn_id);
            expect_response(item.generation);
            sent = length > 0 && (size_t)length < sizeof(event) && send_text(event) == length;
            finish_turn_send(item.generation, sent);
        }

        if (!sent) {
            ESP_LOGW(TAG, "outbound event %d failed", item.type);
            fail_turn(item.generation, "Audio upload interrupted");
            send_text("{\"type\":\"response.cancel\",\"clear_input\":true}");
        }
    }
}

static void abort_recording(uint32_t generation, const char *detail)
{
    fail_turn(generation, detail);
    if (!queue_outbound(OUTBOUND_ABORT, generation, NULL, NULL,
                        pdMS_TO_TICKS(1000), true)) {
        ESP_LOGW(TAG, "could not queue turn abort");
    }
}

static void recording_task(void *arg)
{
    (void)arg;
    control_event_t event;
    uint32_t turn_counter = 0;
    for (;;) {
        if (xQueueReceive(s_controls, &event, portMAX_DELAY) != pdTRUE || event != CONTROL_PRESS) {
            continue;
        }
        if (!websocket_ready()) {
            passport_ui_set(PASSPORT_UI_CONNECTING, "Waiting for Ripple");
            continue;
        }

        invalidate_response();
        uint32_t generation = begin_turn();
        char turn_id[48];
        snprintf(turn_id, sizeof(turn_id), "passport-%lld-%lu",
                 (long long)esp_timer_get_time(), (unsigned long)++turn_counter);
        if (!queue_outbound(OUTBOUND_CANCEL_INPUT, generation, NULL, NULL, 0, false) ||
            !queue_outbound(OUTBOUND_SPEECH_START, generation, turn_id, NULL, 0, false)) {
            abort_recording(generation, "Audio upload busy");
            continue;
        }

        xEventGroupClearBits(s_flags, RESPONSE_DONE);
        xEventGroupSetBits(s_flags, RECORDING);
        passport_ui_set(PASSPORT_UI_LISTENING, "Release OK to send");
        ESP_LOGI(TAG, "PTT started: %s", turn_id);

        xSemaphoreTake(s_audio_lock, portMAX_DELAY);
        if (bsp_audio_set_format(PASSPORT_INPUT_SAMPLE_RATE, 16, 1) != ESP_OK) {
            xSemaphoreGive(s_audio_lock);
            xEventGroupClearBits(s_flags, RECORDING);
            abort_recording(generation, "Microphone unavailable");
            continue;
        }

        int16_t pcm[PASSPORT_INPUT_SAMPLES];
        bool released = (xEventGroupGetBits(s_flags) & PTT_RELEASED) != 0;
        bool failed = false;
        uint32_t chunks = 0;
        while (!released && websocket_ready() && turn_is_valid(generation)) {
            released = (xEventGroupGetBits(s_flags) & PTT_RELEASED) != 0;
            if (released) break;
            if (bsp_audio_read(pcm, sizeof(pcm)) != ESP_OK) {
                failed = true;
                break;
            }
            if (!queue_outbound(OUTBOUND_AUDIO, generation, NULL, pcm, 0, false)) {
                failed = true;
                break;
            }
            chunks++;
        }
        xSemaphoreGive(s_audio_lock);
        xEventGroupClearBits(s_flags, RECORDING);

        if (released && !failed && chunks > 0 && websocket_ready() && turn_is_valid(generation)) {
            if (queue_outbound(OUTBOUND_COMMIT, generation, turn_id, NULL,
                               pdMS_TO_TICKS(1000), false)) {
                ESP_LOGI(TAG, "PTT queued for commit: %s", turn_id);
                passport_ui_set(PASSPORT_UI_THINKING, "Ripple is thinking");
            } else {
                abort_recording(generation, "Audio upload busy");
            }
        } else if (failed || (released && chunks == 0)) {
            abort_recording(generation, failed ? "Audio upload busy" : "Hold OK a little longer");
        } else {
            invalidate_turn();
        }
    }
}

static bool playback_generation_active(uint32_t generation)
{
    portENTER_CRITICAL(&s_state_mux);
    bool active = s_playback_active && s_response_generation == generation;
    portEXIT_CRITICAL(&s_state_mux);
    return active;
}

static void subtract_queued_samples(const audio_chunk_t *chunk)
{
    portENTER_CRITICAL(&s_state_mux);
    if (chunk->generation == s_response_generation) {
        s_queued_samples = s_queued_samples >= chunk->samples
                               ? s_queued_samples - chunk->samples
                               : 0;
    }
    portEXIT_CRITICAL(&s_state_mux);
}

static void playback_task(void *arg)
{
    (void)arg;
    audio_chunk_t chunk;
    bool format_ready = false;
    bool buffering = true;
    uint32_t active_generation = 0;
    uint32_t underruns = 0;
    for (;;) {
        state_snapshot_t state = state_snapshot();
        EventBits_t flags = xEventGroupGetBits(s_flags);
        if (!state.playback_active) {
            format_ready = false;
            buffering = true;
            vTaskDelay(pdMS_TO_TICKS(10));
            continue;
        }
        if (active_generation != state.response_generation) {
            active_generation = state.response_generation;
            format_ready = false;
            buffering = true;
        }

        bool response_finished = (flags & RESPONSE_DONE) != 0;
        uint32_t buffered_ms = passport_audio_duration_ms(
            state.queued_samples, PASSPORT_OUTPUT_SAMPLE_RATE);
        if (buffering) {
            if (buffered_ms >= PASSPORT_PLAYBACK_PREBUFFER_MS ||
                (response_finished && buffered_ms > 0)) {
                buffering = false;
                passport_ui_set(PASSPORT_UI_SPEAKING, "Hold OK to interrupt");
                ESP_LOGI(TAG, "playback started with %lu ms buffered",
                         (unsigned long)buffered_ms);
            } else if (response_finished && buffered_ms == 0) {
                complete_playback(active_generation);
                continue;
            } else {
                vTaskDelay(pdMS_TO_TICKS(10));
                continue;
            }
        }

        if (xQueueReceive(s_audio, &chunk, pdMS_TO_TICKS(100)) == pdTRUE) {
            subtract_queued_samples(&chunk);
            if (chunk.generation == active_generation &&
                playback_generation_active(active_generation)) {
                xSemaphoreTake(s_audio_lock, portMAX_DELAY);
                if (playback_generation_active(active_generation)) {
                    if (!format_ready) {
                        format_ready = bsp_audio_set_format(PASSPORT_OUTPUT_SAMPLE_RATE, 16, 1) == ESP_OK;
                        bsp_audio_set_volume(s_volume);
                    }
                    if (format_ready && bsp_audio_write(chunk.pcm, chunk.bytes) != ESP_OK) {
                        format_ready = false;
                        invalidate_response();
                        passport_ui_set(PASSPORT_UI_ERROR, "Speaker unavailable");
                    }
                }
                xSemaphoreGive(s_audio_lock);
            }
            free(chunk.pcm);

            state = state_snapshot();
            if ((xEventGroupGetBits(s_flags) & RESPONSE_DONE) &&
                state.response_generation == active_generation && state.queued_samples == 0) {
                complete_playback(active_generation);
            }
        } else if (xEventGroupGetBits(s_flags) & RESPONSE_DONE) {
            complete_playback(active_generation);
        } else if (state.accepting) {
            buffering = true;
            ESP_LOGW(TAG, "playback underrun #%lu; rebuffering to %u ms",
                     (unsigned long)++underruns, PASSPORT_PLAYBACK_PREBUFFER_MS);
        }
    }
}

static bool enqueue_audio(const char *encoded, uint32_t sample_rate)
{
    if (!encoded) return false;
    size_t encoded_len = strlen(encoded);
    if (encoded_len == 0 || encoded_len > MAX_OUTPUT_BASE64_BYTES) return false;

    state_snapshot_t state = state_snapshot();
    if (!state.accepting) return false;
    size_t raw_capacity = encoded_len * 3 / 4 + 4;
    if (raw_capacity > PASSPORT_OUTPUT_MAX_SAMPLES * sizeof(float) + 4) return false;
    unsigned char *raw = malloc(raw_capacity);
    if (!raw) return false;
    size_t raw_len = 0;
    if (mbedtls_base64_decode(raw, raw_capacity, &raw_len,
                              (const unsigned char *)encoded, encoded_len) != 0 ||
        raw_len % sizeof(float) != 0) {
        free(raw);
        return false;
    }

    size_t sample_count = raw_len / sizeof(float);
    if (!passport_output_chunk_is_valid(sample_rate, sample_count)) {
        free(raw);
        return false;
    }
    int16_t *pcm = malloc(sample_count * sizeof(int16_t));
    if (!pcm) {
        free(raw);
        return false;
    }
    const float *samples = (const float *)raw;
    for (size_t i = 0; i < sample_count; ++i) {
        float value = isfinite(samples[i]) ? samples[i] : 0.0f;
        value = fmaxf(-1.0f, fminf(1.0f, value));
        pcm[i] = (int16_t)(value * 32767.0f);
    }
    free(raw);

    audio_chunk_t chunk = {
        .pcm = pcm,
        .bytes = sample_count * sizeof(int16_t),
        .samples = sample_count,
        .generation = state.response_generation,
    };

    // Reserve the sample count before publishing the pointer to the playback
    // task. Otherwise a fast consumer can subtract before this task adds it.
    portENTER_CRITICAL(&s_state_mux);
    bool current = s_playback_active && s_response_generation == chunk.generation;
    if (current) {
        s_queued_samples += sample_count;
        s_response_deadline_us = esp_timer_get_time() + RESPONSE_IDLE_TIMEOUT_US;
    }
    portEXIT_CRITICAL(&s_state_mux);
    if (!current) {
        free(pcm);
        return false;
    }
    if (xQueueSend(s_audio, &chunk, 0) != pdTRUE) {
        portENTER_CRITICAL(&s_state_mux);
        if (s_response_generation == chunk.generation) {
            s_queued_samples = s_queued_samples >= sample_count
                                   ? s_queued_samples - sample_count
                                   : 0;
        }
        portEXIT_CRITICAL(&s_state_mux);
        free(pcm);
        return false;
    }
    return true;
}

static const char *json_string(cJSON *root, const char *name)
{
    cJSON *item = cJSON_GetObjectItemCaseSensitive(root, name);
    return cJSON_IsString(item) ? item->valuestring : NULL;
}

static bool validate_session_ready(cJSON *root)
{
    cJSON *protocol = cJSON_GetObjectItemCaseSensitive(root, "protocol_version");
    cJSON *input_rate = cJSON_GetObjectItemCaseSensitive(root, "sample_rate_in");
    cJSON *output_rate = cJSON_GetObjectItemCaseSensitive(root, "sample_rate_out");
    return (!cJSON_IsNumber(protocol) || protocol->valueint == 5) &&
           (!cJSON_IsNumber(input_rate) || input_rate->valueint == PASSPORT_INPUT_SAMPLE_RATE) &&
           (!cJSON_IsNumber(output_rate) || output_rate->valueint == PASSPORT_OUTPUT_SAMPLE_RATE);
}

static void handle_message(char *message)
{
    cJSON *root = cJSON_Parse(message);
    if (!root) {
        ESP_LOGW(TAG, "invalid JSON event ignored");
        return;
    }
    const char *kind = json_string(root, "type");
    if (!kind) kind = "";
    const char *response_id = json_string(root, "response_id");

    if (strcmp(kind, "session.ready") == 0) {
        if (!validate_session_ready(root)) {
            passport_ui_set(PASSPORT_UI_ERROR, "Gateway audio settings mismatch");
            ESP_LOGE(TAG, "session.ready protocol or sample rate mismatch");
        } else {
            portENTER_CRITICAL(&s_state_mux);
            s_session_deadline_us = 0;
            portEXIT_CRITICAL(&s_state_mux);
            xEventGroupSetBits(s_flags, SESSION_READY);
            passport_ui_set(PASSPORT_UI_READY, "Connected");
            ESP_LOGI(TAG, "session ready (free heap: %lu)",
                     (unsigned long)esp_get_free_heap_size());
        }
    } else if (strcmp(kind, "response.created") == 0) {
        if (begin_response(response_id)) {
            passport_ui_set(PASSPORT_UI_THINKING, "Ripple is thinking");
            ESP_LOGI(TAG, "response created: %s", response_id);
        } else {
            ESP_LOGW(TAG, "unexpected response.created ignored");
            queue_outbound(OUTBOUND_ABORT, 0, NULL, NULL, 0, true);
        }
    } else if (strcmp(kind, "response.audio.delta") == 0) {
        cJSON *rate = cJSON_GetObjectItemCaseSensitive(root, "sample_rate");
        const char *audio = json_string(root, "audio");
        if (!response_matches(response_id)) {
            ESP_LOGW(TAG, "stale response audio ignored");
        } else if (!cJSON_IsNumber(rate) || !enqueue_audio(audio, (uint32_t)rate->valueint)) {
            invalidate_response();
            passport_ui_set(PASSPORT_UI_ERROR, "Invalid or overflowing audio stream");
            queue_outbound(OUTBOUND_ABORT, 0, NULL, NULL, 0, true);
        }
    } else if (strcmp(kind, "response.done") == 0) {
        if (response_matches(response_id)) {
            mark_response_done();
            ESP_LOGI(TAG, "response done (minimum free heap: %lu)",
                     (unsigned long)esp_get_minimum_free_heap_size());
        }
    } else if (strcmp(kind, "response.cancelled") == 0) {
        state_snapshot_t state = state_snapshot();
        if (response_matches(response_id) ||
            (state.waiting_response && passport_response_id_is_valid(response_id))) {
            invalidate_response();
            if (websocket_ready() && !(xEventGroupGetBits(s_flags) & RECORDING)) {
                passport_ui_set(PASSPORT_UI_READY, "Connected");
            }
        }
    } else if (strcmp(kind, "response.failed") == 0) {
        state_snapshot_t state = state_snapshot();
        if (response_matches(response_id) ||
            (state.waiting_response && passport_response_id_is_valid(response_id))) {
            const char *detail = json_string(root, "message");
            invalidate_response();
            passport_ui_set(PASSPORT_UI_ERROR, detail ? detail : "Request failed");
            ESP_LOGE(TAG, "response failed: %s", detail ? detail : "Request failed");
        }
    } else if (strcmp(kind, "error") == 0) {
        const char *detail = json_string(root, "message");
        invalidate_response();
        invalidate_turn();
        passport_ui_set(PASSPORT_UI_ERROR, detail ? detail : "Gateway request failed");
        ESP_LOGE(TAG, "gateway error: %s", detail ? detail : "Gateway request failed");
    }
    cJSON_Delete(root);
}

static void reset_message_assembly(void)
{
    free(s_message);
    s_message = NULL;
    s_message_capacity = 0;
    s_message_received = 0;
    s_frame_base = 0;
    s_message_size = 0;
    s_fragment_active = false;
    s_discard_message = false;
}

static bool begin_message_frame(const esp_websocket_event_data_t *data)
{
    if (data->payload_len <= 0 || data->payload_len > MAX_WS_MESSAGE_BYTES) return false;
    bool continuation = data->op_code == 0x0;
    if (!continuation) {
        reset_message_assembly();
        s_fragment_active = data->fin == 0;
        s_message_capacity = s_fragment_active ? MAX_WS_MESSAGE_BYTES
                                               : (size_t)data->payload_len;
        if (s_message_capacity > MAX_WS_MESSAGE_BYTES) return false;
        s_message = malloc(s_message_capacity + 1);
        if (!s_message) return false;
        s_frame_base = 0;
        s_message_size = (size_t)data->payload_len;
        return true;
    }

    if (!s_fragment_active || !s_message) return false;
    s_frame_base = s_message_received;
    if ((size_t)data->payload_len > MAX_WS_MESSAGE_BYTES - s_frame_base) return false;
    s_message_size = s_frame_base + (size_t)data->payload_len;
    return true;
}

static void websocket_event(void *arg, esp_event_base_t base, int32_t id, void *event_data)
{
    (void)arg;
    (void)base;
    esp_websocket_event_data_t *data = event_data;
    if (id == WEBSOCKET_EVENT_CONNECTED) {
        ESP_LOGI(TAG, "websocket connected");
        portENTER_CRITICAL(&s_state_mux);
        s_session_deadline_us = esp_timer_get_time() + SESSION_READY_TIMEOUT_US;
        portEXIT_CRITICAL(&s_state_mux);
        passport_ui_set(PASSPORT_UI_CONNECTING, "Starting session");
        if (!queue_outbound(OUTBOUND_SESSION_START, 0, NULL, NULL, 0, true)) {
            passport_ui_set(PASSPORT_UI_ERROR, "Session startup queue full");
        }
    } else if (id == WEBSOCKET_EVENT_DISCONNECTED) {
        ESP_LOGW(TAG, "websocket disconnected");
        xEventGroupClearBits(s_flags, SESSION_READY | RECORDING | RESPONSE_DONE);
        portENTER_CRITICAL(&s_state_mux);
        s_session_deadline_us = 0;
        portEXIT_CRITICAL(&s_state_mux);
        invalidate_turn();
        invalidate_response();
        reset_message_assembly();
        passport_ui_set(PASSPORT_UI_CONNECTING, "Reconnecting to Ripple");
    } else if (id == WEBSOCKET_EVENT_ERROR) {
        ESP_LOGE(TAG, "websocket error");
        passport_ui_set(PASSPORT_UI_ERROR, "Gateway connection error");
    } else if (id == WEBSOCKET_EVENT_DATA && data &&
               (data->op_code == 0x1 || data->op_code == 0x0)) {
        if (data->payload_offset == 0) {
            if (!begin_message_frame(data)) {
                free(s_message);
                s_message = NULL;
                s_message_capacity = 0;
                s_discard_message = true;
                ESP_LOGW(TAG, "invalid or oversized WebSocket message ignored");
            }
        }
        if (s_discard_message) {
            size_t offset = data->payload_offset < 0 ? 0 : (size_t)data->payload_offset;
            size_t length = data->data_len < 0 ? 0 : (size_t)data->data_len;
            size_t payload = data->payload_len < 0 ? 0 : (size_t)data->payload_len;
            if (data->fin && offset <= payload && length >= payload - offset) {
                reset_message_assembly();
            }
            return;
        }
        if (!s_message || data->payload_offset < 0 || data->data_len < 0 ||
            s_frame_base > s_message_size ||
            (size_t)data->payload_offset > s_message_size - s_frame_base ||
            (size_t)data->data_len > s_message_size - s_frame_base - (size_t)data->payload_offset) {
            reset_message_assembly();
            return;
        }
        size_t destination = s_frame_base + (size_t)data->payload_offset;
        memcpy(s_message + destination, data->data_ptr, data->data_len);
        if ((size_t)data->payload_offset + (size_t)data->data_len ==
            (size_t)data->payload_len) {
            s_message_received = s_message_size;
            if (data->fin) {
                s_message[s_message_received] = '\0';
                handle_message(s_message);
                reset_message_assembly();
            } else {
                s_fragment_active = true;
            }
        }
    }
}

static void supervisor_task(void *arg)
{
    (void)arg;
    for (;;) {
        vTaskDelay(pdMS_TO_TICKS(500));
        int64_t now = esp_timer_get_time();
        state_snapshot_t state = state_snapshot();
        if (passport_deadline_expired(now, state.session_deadline_us) &&
            s_client && esp_websocket_client_is_connected(s_client)) {
            ESP_LOGW(TAG, "session.ready timeout; restarting WebSocket");
            portENTER_CRITICAL(&s_state_mux);
            s_session_deadline_us = 0;
            portEXIT_CRITICAL(&s_state_mux);
            passport_ui_set(PASSPORT_UI_ERROR, "Gateway handshake timed out");
            esp_websocket_client_stop(s_client);
            vTaskDelay(pdMS_TO_TICKS(250));
            esp_err_t err = esp_websocket_client_start(s_client);
            if (err != ESP_OK) {
                ESP_LOGE(TAG, "WebSocket restart failed: %s", esp_err_to_name(err));
            }
            continue;
        }
        if (passport_deadline_expired(now, state.response_deadline_us)) {
            ESP_LOGW(TAG, "response timeout");
            invalidate_response();
            invalidate_turn();
            queue_outbound(OUTBOUND_ABORT, 0, NULL, NULL, 0, true);
            passport_ui_set(PASSPORT_UI_ERROR, "Ripple response timed out");
        }
    }
}

static void delete_task(TaskHandle_t *task)
{
    if (*task) {
        vTaskDelete(*task);
        *task = NULL;
    }
}

static void cleanup_start_failure(void)
{
    delete_task(&s_supervisor_task);
    delete_task(&s_playback_task);
    delete_task(&s_recording_task);
    delete_task(&s_network_task);
    if (s_client) {
        esp_websocket_client_destroy(s_client);
        s_client = NULL;
    }
    if (s_audio_lock) vSemaphoreDelete(s_audio_lock);
    if (s_audio) vQueueDelete(s_audio);
    if (s_outbound) vQueueDelete(s_outbound);
    if (s_controls) vQueueDelete(s_controls);
    if (s_flags) vEventGroupDelete(s_flags);
    s_audio_lock = NULL;
    s_audio = NULL;
    s_outbound = NULL;
    s_controls = NULL;
    s_flags = NULL;
}

esp_err_t passport_realtime_start(const char *gateway)
{
    if (!passport_gateway_is_valid(gateway)) return ESP_ERR_INVALID_ARG;
    if (s_client) return ESP_ERR_INVALID_STATE;
    s_flags = xEventGroupCreate();
    s_controls = xQueueCreate(8, sizeof(control_event_t));
    s_outbound = xQueueCreate(OUTBOUND_QUEUE_ITEMS, sizeof(outbound_item_t));
    s_audio = xQueueCreate(PLAYBACK_QUEUE_CHUNKS, sizeof(audio_chunk_t));
    s_audio_lock = xSemaphoreCreateMutex();
    if (!s_flags || !s_controls || !s_outbound || !s_audio || !s_audio_lock) {
        cleanup_start_failure();
        return ESP_ERR_NO_MEM;
    }

    char uri[160];
    int uri_length = snprintf(uri, sizeof(uri), "ws://%s/v1/agent/realtime", gateway);
    if (uri_length <= 0 || (size_t)uri_length >= sizeof(uri)) {
        cleanup_start_failure();
        return ESP_ERR_INVALID_SIZE;
    }
    esp_websocket_client_config_t config = {
        .uri = uri,
        .buffer_size = 2048,
        .task_stack = 6144,
        .network_timeout_ms = 10000,
        .reconnect_timeout_ms = 3000,
    };
    s_client = esp_websocket_client_init(&config);
    if (!s_client) {
        cleanup_start_failure();
        return ESP_ERR_NO_MEM;
    }
    esp_err_t err = esp_websocket_register_events(
        s_client, WEBSOCKET_EVENT_ANY, websocket_event, NULL);
    if (err != ESP_OK ||
        xTaskCreate(network_task, "audio_upload", 6144, NULL, 6, &s_network_task) != pdPASS ||
        xTaskCreate(recording_task, "recording", 5120, NULL, 6, &s_recording_task) != pdPASS ||
        xTaskCreate(playback_task, "playback", 4608, NULL, 6, &s_playback_task) != pdPASS ||
        xTaskCreate(supervisor_task, "realtime_watch", 3072, NULL, 5, &s_supervisor_task) != pdPASS) {
        cleanup_start_failure();
        return err != ESP_OK ? err : ESP_ERR_NO_MEM;
    }

    ESP_LOGI(TAG, "connecting %s", uri);
    err = esp_websocket_client_start(s_client);
    if (err != ESP_OK) cleanup_start_failure();
    return err;
}

void passport_realtime_ptt_press(void)
{
    if (!s_controls) return;
    xEventGroupClearBits(s_flags, PTT_RELEASED);
    control_event_t event = CONTROL_PRESS;
    xQueueSend(s_controls, &event, 0);
}

void passport_realtime_ptt_release(void)
{
    if (!s_flags) return;
    xEventGroupSetBits(s_flags, PTT_RELEASED);
}

void passport_realtime_set_volume(uint8_t percent)
{
    s_volume = percent > 100 ? 100 : percent;
    if (!s_audio_lock || (s_flags && (xEventGroupGetBits(s_flags) & RECORDING))) return;
    if (xSemaphoreTake(s_audio_lock, pdMS_TO_TICKS(200)) == pdTRUE) {
        bsp_audio_set_volume(s_volume);
        xSemaphoreGive(s_audio_lock);
    }
}

uint8_t passport_realtime_get_volume(void)
{
    return s_volume;
}

bool passport_realtime_is_ready(void)
{
    return websocket_ready();
}
