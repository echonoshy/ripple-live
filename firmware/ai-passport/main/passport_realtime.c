#include "passport_realtime.h"

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
#define INPUT_SAMPLES 640
#define PLAYBACK_CHUNK_MS 100
#define PLAYBACK_PREBUFFER_MS 400
#define PLAYBACK_PREBUFFER_CHUNKS (PLAYBACK_PREBUFFER_MS / PLAYBACK_CHUNK_MS)
#define PLAYBACK_QUEUE_CHUNKS 8

typedef enum { CONTROL_PRESS = 1, CONTROL_RELEASE = 2 } control_event_t;

typedef struct {
    int16_t *pcm;
    size_t bytes;
} audio_chunk_t;

static const char *TAG = "passport_ws";
static esp_websocket_client_handle_t s_client;
static EventGroupHandle_t s_flags;
static QueueHandle_t s_controls;
static QueueHandle_t s_audio;
static SemaphoreHandle_t s_audio_lock;
static volatile bool s_accept_audio;
static volatile uint8_t s_volume = 70;
static char *s_message;
static size_t s_message_size;

static bool websocket_ready(void)
{
    return s_client && esp_websocket_client_is_connected(s_client) &&
           (xEventGroupGetBits(s_flags) & SESSION_READY);
}

static int send_text(const char *text)
{
    if (!s_client || !esp_websocket_client_is_connected(s_client)) return -1;
    return esp_websocket_client_send_text(s_client, text, strlen(text), pdMS_TO_TICKS(5000));
}

static void clear_audio_queue(void)
{
    audio_chunk_t chunk;
    while (xQueueReceive(s_audio, &chunk, 0) == pdTRUE) free(chunk.pcm);
}

static void send_audio(const int16_t *pcm)
{
    float samples[INPUT_SAMPLES];
    for (size_t i = 0; i < INPUT_SAMPLES; ++i) samples[i] = pcm[i] / 32768.0f;

    size_t encoded_size = 4 * ((sizeof(samples) + 2) / 3) + 1;
    unsigned char *encoded = malloc(encoded_size);
    if (!encoded) return;
    size_t encoded_len = 0;
    if (mbedtls_base64_encode(encoded, encoded_size, &encoded_len,
                              (const unsigned char *)samples, sizeof(samples)) != 0) {
        free(encoded);
        return;
    }
    char *event = malloc(encoded_len + 96);
    if (event) {
        int length = snprintf(event, encoded_len + 96,
                              "{\"type\":\"input.audio.append\",\"sample_rate\":16000,\"audio\":\"%.*s\"}",
                              (int)encoded_len, encoded);
        if (length > 0) send_text(event);
        free(event);
    }
    free(encoded);
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
        if (!websocket_ready()) continue;

        s_accept_audio = false;
        clear_audio_queue();
        send_text("{\"type\":\"response.cancel\",\"clear_input\":true}");
        xEventGroupClearBits(s_flags, RESPONSE_DONE);
        xEventGroupSetBits(s_flags, RECORDING);
        passport_ui_set(PASSPORT_UI_LISTENING, "Release OK to send");

        char turn_id[48];
        snprintf(turn_id, sizeof(turn_id), "passport-%lld-%lu",
                 (long long)esp_timer_get_time(), (unsigned long)++turn_counter);
        char start_event[128];
        snprintf(start_event, sizeof(start_event),
                 "{\"type\":\"input.speech_started\",\"turn_id\":\"%s\"}", turn_id);
        send_text(start_event);
        ESP_LOGI(TAG, "PTT started: %s", turn_id);

        xSemaphoreTake(s_audio_lock, portMAX_DELAY);
        if (bsp_audio_set_format(16000, 16, 1) != ESP_OK) {
            xSemaphoreGive(s_audio_lock);
            xEventGroupClearBits(s_flags, RECORDING);
            passport_ui_set(PASSPORT_UI_ERROR, "Microphone unavailable");
            continue;
        }

        int16_t pcm[INPUT_SAMPLES];
        bool released = false;
        while (!released && websocket_ready()) {
            if (bsp_audio_read(pcm, sizeof(pcm)) != ESP_OK) break;
            send_audio(pcm);
            while (xQueueReceive(s_controls, &event, 0) == pdTRUE) {
                if (event == CONTROL_RELEASE) released = true;
            }
        }
        xSemaphoreGive(s_audio_lock);
        xEventGroupClearBits(s_flags, RECORDING);

        if (released && websocket_ready()) {
            char commit[112];
            snprintf(commit, sizeof(commit),
                     "{\"type\":\"input.commit\",\"turn_id\":\"%s\"}", turn_id);
            send_text(commit);
            ESP_LOGI(TAG, "PTT committed: %s", turn_id);
            passport_ui_set(PASSPORT_UI_THINKING, "Ripple is thinking");
        }
    }
}

static void playback_task(void *arg)
{
    (void)arg;
    audio_chunk_t chunk;
    bool format_ready = false;
    bool buffering = true;
    uint32_t underruns = 0;
    for (;;) {
        if (!s_accept_audio) {
            format_ready = false;
            buffering = true;
        }

        EventBits_t flags = xEventGroupGetBits(s_flags);
        UBaseType_t queued = uxQueueMessagesWaiting(s_audio);
        if (buffering) {
            bool response_finished = (flags & RESPONSE_DONE) != 0;
            if (s_accept_audio &&
                (queued >= PLAYBACK_PREBUFFER_CHUNKS || (response_finished && queued > 0))) {
                buffering = false;
                ESP_LOGI(TAG, "playback started with %lu ms buffered",
                         (unsigned long)queued * PLAYBACK_CHUNK_MS);
            } else {
                if (response_finished && queued == 0) {
                    xEventGroupClearBits(s_flags, RESPONSE_DONE);
                    format_ready = false;
                    if (websocket_ready() && !(flags & RECORDING)) {
                        passport_ui_set(PASSPORT_UI_READY, "Connected");
                    }
                }
                vTaskDelay(pdMS_TO_TICKS(10));
                continue;
            }
        }

        if (xQueueReceive(s_audio, &chunk, pdMS_TO_TICKS(100)) == pdTRUE) {
            if (s_accept_audio) {
                xSemaphoreTake(s_audio_lock, portMAX_DELAY);
                if (!format_ready) {
                    format_ready = bsp_audio_set_format(24000, 16, 1) == ESP_OK;
                    bsp_audio_set_volume(s_volume);
                }
                if (format_ready) {
                    passport_ui_set(PASSPORT_UI_SPEAKING, "Hold OK to interrupt");
                    bsp_audio_write(chunk.pcm, chunk.bytes);
                }
                xSemaphoreGive(s_audio_lock);
            }
            free(chunk.pcm);
        } else if (xEventGroupGetBits(s_flags) & RESPONSE_DONE) {
            xEventGroupClearBits(s_flags, RESPONSE_DONE);
            format_ready = false;
            buffering = true;
            if (websocket_ready() && !(xEventGroupGetBits(s_flags) & RECORDING)) {
                passport_ui_set(PASSPORT_UI_READY, "Connected");
            }
        } else if (s_accept_audio) {
            buffering = true;
            ESP_LOGW(TAG, "playback underrun #%lu; buffering %u chunks",
                     (unsigned long)++underruns, PLAYBACK_PREBUFFER_CHUNKS);
        }
    }
}

static void enqueue_audio(const char *encoded)
{
    if (!s_accept_audio || !encoded) return;
    size_t raw_capacity = strlen(encoded) * 3 / 4 + 4;
    unsigned char *raw = malloc(raw_capacity);
    if (!raw) return;
    size_t raw_len = 0;
    if (mbedtls_base64_decode(raw, raw_capacity, &raw_len,
                              (const unsigned char *)encoded, strlen(encoded)) != 0 ||
        raw_len % sizeof(float) != 0) {
        free(raw);
        return;
    }

    size_t sample_count = raw_len / sizeof(float);
    int16_t *pcm = malloc(sample_count * sizeof(int16_t));
    if (!pcm) {
        free(raw);
        return;
    }
    const float *samples = (const float *)raw;
    for (size_t i = 0; i < sample_count; ++i) {
        float value = fmaxf(-1.0f, fminf(1.0f, samples[i]));
        pcm[i] = (int16_t)(value * 32767.0f);
    }
    free(raw);

    audio_chunk_t chunk = {.pcm = pcm, .bytes = sample_count * sizeof(int16_t)};
    if (xQueueSend(s_audio, &chunk, pdMS_TO_TICKS(100)) != pdTRUE) {
        free(pcm);
        passport_ui_set(PASSPORT_UI_ERROR, "Audio buffer overflow");
    }
}

static void handle_message(char *message)
{
    cJSON *root = cJSON_Parse(message);
    if (!root) return;
    cJSON *type = cJSON_GetObjectItemCaseSensitive(root, "type");
    const char *kind = cJSON_IsString(type) ? type->valuestring : "";

    if (strcmp(kind, "session.ready") == 0) {
        xEventGroupSetBits(s_flags, SESSION_READY);
        passport_ui_set(PASSPORT_UI_READY, "Connected");
        ESP_LOGI(TAG, "session ready (free heap: %lu)",
                 (unsigned long)esp_get_free_heap_size());
    } else if (strcmp(kind, "response.created") == 0) {
        s_accept_audio = true;
        xEventGroupClearBits(s_flags, RESPONSE_DONE);
        passport_ui_set(PASSPORT_UI_THINKING, "Ripple is thinking");
        ESP_LOGI(TAG, "response created");
    } else if (strcmp(kind, "response.audio.delta") == 0) {
        cJSON *audio = cJSON_GetObjectItemCaseSensitive(root, "audio");
        if (cJSON_IsString(audio)) enqueue_audio(audio->valuestring);
    } else if (strcmp(kind, "response.done") == 0) {
        xEventGroupSetBits(s_flags, RESPONSE_DONE);
        ESP_LOGI(TAG, "response done (minimum free heap: %lu)",
                 (unsigned long)esp_get_minimum_free_heap_size());
    } else if (strcmp(kind, "response.cancelled") == 0) {
        s_accept_audio = false;
        clear_audio_queue();
    } else if (strcmp(kind, "response.failed") == 0 || strcmp(kind, "error") == 0) {
        cJSON *message_item = cJSON_GetObjectItemCaseSensitive(root, "message");
        passport_ui_set(PASSPORT_UI_ERROR,
                        cJSON_IsString(message_item) ? message_item->valuestring : "Request failed");
        ESP_LOGE(TAG, "%s: %s", kind,
                 cJSON_IsString(message_item) ? message_item->valuestring : "Request failed");
    }
    cJSON_Delete(root);
}

static void websocket_event(void *arg, esp_event_base_t base, int32_t id, void *event_data)
{
    (void)arg;
    (void)base;
    esp_websocket_event_data_t *data = event_data;
    if (id == WEBSOCKET_EVENT_CONNECTED) {
        ESP_LOGI(TAG, "websocket connected");
        passport_ui_set(PASSPORT_UI_CONNECTING, "Starting session");
        send_text("{\"type\":\"session.start\",\"protocol_version\":5,\"client_build\":\"passport-0.1\",\"mode\":\"audio\"}");
    } else if (id == WEBSOCKET_EVENT_DISCONNECTED) {
        ESP_LOGW(TAG, "websocket disconnected");
        xEventGroupClearBits(s_flags, SESSION_READY | RECORDING);
        s_accept_audio = false;
        clear_audio_queue();
        passport_ui_set(PASSPORT_UI_CONNECTING, "Reconnecting to Ripple");
    } else if (id == WEBSOCKET_EVENT_ERROR) {
        ESP_LOGE(TAG, "websocket error");
        passport_ui_set(PASSPORT_UI_ERROR, "Gateway connection error");
    } else if (id == WEBSOCKET_EVENT_DATA && data &&
               (data->op_code == 0x1 || data->op_code == 0x0)) {
        if (data->payload_offset == 0) {
            free(s_message);
            s_message_size = data->payload_len;
            s_message = malloc(s_message_size + 1);
        }
        if (!s_message || data->payload_offset + data->data_len > s_message_size) return;
        memcpy(s_message + data->payload_offset, data->data_ptr, data->data_len);
        if (data->payload_offset + data->data_len == s_message_size) {
            s_message[s_message_size] = '\0';
            handle_message(s_message);
            free(s_message);
            s_message = NULL;
            s_message_size = 0;
        }
    }
}

esp_err_t passport_realtime_start(const char *gateway)
{
    if (!gateway || !gateway[0]) return ESP_ERR_INVALID_ARG;
    s_flags = xEventGroupCreate();
    s_controls = xQueueCreate(8, sizeof(control_event_t));
    s_audio = xQueueCreate(PLAYBACK_QUEUE_CHUNKS, sizeof(audio_chunk_t));
    s_audio_lock = xSemaphoreCreateMutex();
    if (!s_flags || !s_controls || !s_audio || !s_audio_lock) return ESP_ERR_NO_MEM;

    char uri[160];
    snprintf(uri, sizeof(uri), "ws://%s/v1/agent/realtime", gateway);
    esp_websocket_client_config_t config = {
        .uri = uri,
        .buffer_size = 2048,
        .task_stack = 6144,
        .network_timeout_ms = 10000,
        .reconnect_timeout_ms = 3000,
    };
    s_client = esp_websocket_client_init(&config);
    if (!s_client) return ESP_ERR_NO_MEM;
    ESP_ERROR_CHECK(esp_websocket_register_events(
        s_client, WEBSOCKET_EVENT_ANY, websocket_event, NULL));
    if (xTaskCreate(recording_task, "recording", 7168, NULL, 6, NULL) != pdPASS ||
        xTaskCreate(playback_task, "playback", 5120, NULL, 6, NULL) != pdPASS) {
        return ESP_ERR_NO_MEM;
    }
    ESP_LOGI(TAG, "connecting %s", uri);
    return esp_websocket_client_start(s_client);
}

void passport_realtime_ptt_press(void)
{
    if (!s_controls) return;
    control_event_t event = CONTROL_PRESS;
    xQueueSend(s_controls, &event, 0);
}

void passport_realtime_ptt_release(void)
{
    if (!s_controls) return;
    control_event_t event = CONTROL_RELEASE;
    xQueueSend(s_controls, &event, 0);
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
