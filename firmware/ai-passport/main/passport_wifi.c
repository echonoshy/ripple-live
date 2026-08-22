#include "passport_wifi.h"

#include "passport_ui.h"
#include "esp_event.h"
#include "esp_http_server.h"
#include "esp_log.h"
#include "esp_mac.h"
#include "esp_netif.h"
#include "esp_system.h"
#include "esp_wifi.h"
#include "freertos/FreeRTOS.h"
#include "freertos/event_groups.h"
#include "freertos/task.h"
#include "nvs.h"
#include <ctype.h>
#include <stdio.h>
#include <string.h>

#define WIFI_CONNECTED BIT0
#define CONFIG_NAMESPACE "ripple"

static const char *TAG = "passport_wifi";
static EventGroupHandle_t s_wifi_events;

static void wifi_event(void *arg, esp_event_base_t base, int32_t id, void *data)
{
    (void)arg;
    (void)data;
    if (base == WIFI_EVENT && id == WIFI_EVENT_STA_DISCONNECTED) {
        xEventGroupClearBits(s_wifi_events, WIFI_CONNECTED);
        passport_ui_set(PASSPORT_UI_CONNECTING, "Reconnecting Wi-Fi");
        esp_wifi_connect();
    } else if (base == IP_EVENT && id == IP_EVENT_STA_GOT_IP) {
        xEventGroupSetBits(s_wifi_events, WIFI_CONNECTED);
    }
}

static bool load_string(nvs_handle_t nvs, const char *key, char *value, size_t size)
{
    size_t needed = size;
    return nvs_get_str(nvs, key, value, &needed) == ESP_OK && value[0] != '\0';
}

static int hex_value(char c)
{
    if (c >= '0' && c <= '9') return c - '0';
    c = (char)tolower((unsigned char)c);
    if (c >= 'a' && c <= 'f') return c - 'a' + 10;
    return -1;
}

static void url_decode(char *output, size_t output_size, const char *input, size_t input_len)
{
    size_t written = 0;
    for (size_t i = 0; i < input_len && written + 1 < output_size; ++i) {
        if (input[i] == '+') {
            output[written++] = ' ';
        } else if (input[i] == '%' && i + 2 < input_len) {
            int hi = hex_value(input[i + 1]);
            int lo = hex_value(input[i + 2]);
            if (hi >= 0 && lo >= 0) {
                output[written++] = (char)((hi << 4) | lo);
                i += 2;
            }
        } else {
            output[written++] = input[i];
        }
    }
    output[written] = '\0';
}

static bool form_value(const char *body, const char *name, char *value, size_t value_size)
{
    size_t name_len = strlen(name);
    const char *cursor = body;
    while (cursor && *cursor) {
        if (strncmp(cursor, name, name_len) == 0 && cursor[name_len] == '=') {
            const char *start = cursor + name_len + 1;
            const char *end = strchr(start, '&');
            url_decode(value, value_size, start, end ? (size_t)(end - start) : strlen(start));
            return value[0] != '\0';
        }
        cursor = strchr(cursor, '&');
        if (cursor) cursor++;
    }
    return false;
}

static esp_err_t setup_page(httpd_req_t *request)
{
    static const char page[] =
        "<!doctype html><meta name=viewport content='width=device-width'>"
        "<style>body{font:16px sans-serif;max-width:420px;margin:40px auto;padding:20px}"
        "input,button{box-sizing:border-box;width:100%;padding:12px;margin:7px 0}</style>"
        "<h1>Ripple Passport</h1><p>Connect this device to Wi-Fi.</p>"
        "<form method=post action=/configure>"
        "<input name=ssid placeholder='Wi-Fi name' required>"
        "<input name=password type=password placeholder='Wi-Fi password'>"
        "<input name=gateway value='140.143.229.103:8700' required>"
        "<button>Save and restart</button></form>";
    httpd_resp_set_type(request, "text/html");
    return httpd_resp_send(request, page, HTTPD_RESP_USE_STRLEN);
}

static void delayed_restart(void *arg)
{
    (void)arg;
    vTaskDelay(pdMS_TO_TICKS(1200));
    esp_restart();
}

static esp_err_t save_config(httpd_req_t *request)
{
    if (request->content_len <= 0 || request->content_len > 512) {
        return httpd_resp_send_err(request, HTTPD_400_BAD_REQUEST, "Invalid form");
    }
    char body[513] = {0};
    int total = 0;
    while (total < request->content_len) {
        int received = httpd_req_recv(request, body + total, request->content_len - total);
        if (received <= 0) return ESP_FAIL;
        total += received;
    }
    body[total] = '\0';

    char ssid[33] = {0};
    char password[65] = {0};
    char gateway[96] = {0};
    if (!form_value(body, "ssid", ssid, sizeof(ssid)) ||
        !form_value(body, "gateway", gateway, sizeof(gateway))) {
        return httpd_resp_send_err(request, HTTPD_400_BAD_REQUEST, "SSID and gateway required");
    }
    (void)form_value(body, "password", password, sizeof(password));

    nvs_handle_t nvs;
    ESP_ERROR_CHECK(nvs_open(CONFIG_NAMESPACE, NVS_READWRITE, &nvs));
    ESP_ERROR_CHECK(nvs_set_str(nvs, "ssid", ssid));
    ESP_ERROR_CHECK(nvs_set_str(nvs, "password", password));
    ESP_ERROR_CHECK(nvs_set_str(nvs, "gateway", gateway));
    ESP_ERROR_CHECK(nvs_commit(nvs));
    nvs_close(nvs);

    passport_ui_set(PASSPORT_UI_CONNECTING, "Saved. Restarting...");
    httpd_resp_sendstr(request, "Saved. The Passport is restarting.");
    xTaskCreate(delayed_restart, "restart", 2048, NULL, 4, NULL);
    return ESP_OK;
}

static esp_err_t start_provisioning(void)
{
    uint8_t mac[6];
    ESP_ERROR_CHECK(esp_read_mac(mac, ESP_MAC_WIFI_SOFTAP));
    char ap_name[33];
    snprintf(ap_name, sizeof(ap_name), "Ripple-Passport-%02X%02X", mac[4], mac[5]);

    esp_netif_create_default_wifi_ap();
    wifi_init_config_t init = WIFI_INIT_CONFIG_DEFAULT();
    ESP_ERROR_CHECK(esp_wifi_init(&init));
    wifi_config_t config = {0};
    config.ap.ssid_len = strlen(ap_name);
    memcpy(config.ap.ssid, ap_name, config.ap.ssid_len);
    config.ap.channel = 1;
    config.ap.max_connection = 4;
    config.ap.authmode = WIFI_AUTH_OPEN;
    ESP_ERROR_CHECK(esp_wifi_set_mode(WIFI_MODE_AP));
    ESP_ERROR_CHECK(esp_wifi_set_config(WIFI_IF_AP, &config));
    ESP_ERROR_CHECK(esp_wifi_start());

    httpd_config_t server_config = HTTPD_DEFAULT_CONFIG();
    httpd_handle_t server = NULL;
    ESP_ERROR_CHECK(httpd_start(&server, &server_config));
    const httpd_uri_t root = {.uri = "/", .method = HTTP_GET, .handler = setup_page};
    const httpd_uri_t configure = {
        .uri = "/configure", .method = HTTP_POST, .handler = save_config};
    ESP_ERROR_CHECK(httpd_register_uri_handler(server, &root));
    ESP_ERROR_CHECK(httpd_register_uri_handler(server, &configure));

    char detail[80];
    snprintf(detail, sizeof(detail), "%s\nOpen 192.168.4.1", ap_name);
    passport_ui_set(PASSPORT_UI_SETUP, detail);
    ESP_LOGI(TAG, "provisioning AP %s", ap_name);
    return ESP_ERR_NOT_FINISHED;
}

esp_err_t passport_wifi_start(char *gateway, size_t gateway_size)
{
    ESP_ERROR_CHECK(esp_netif_init());
    ESP_ERROR_CHECK(esp_event_loop_create_default());
    s_wifi_events = xEventGroupCreate();

    nvs_handle_t nvs;
    if (nvs_open(CONFIG_NAMESPACE, NVS_READONLY, &nvs) != ESP_OK) {
        return start_provisioning();
    }
    char ssid[33] = {0};
    char password[65] = {0};
    bool configured = load_string(nvs, "ssid", ssid, sizeof(ssid)) &&
                      load_string(nvs, "gateway", gateway, gateway_size);
    size_t password_size = sizeof(password);
    if (nvs_get_str(nvs, "password", password, &password_size) != ESP_OK) password[0] = '\0';
    nvs_close(nvs);
    if (!configured) return start_provisioning();

    passport_ui_set(PASSPORT_UI_CONNECTING, "Connecting Wi-Fi");
    esp_netif_create_default_wifi_sta();
    wifi_init_config_t init = WIFI_INIT_CONFIG_DEFAULT();
    ESP_ERROR_CHECK(esp_wifi_init(&init));
    ESP_ERROR_CHECK(esp_event_handler_register(WIFI_EVENT, ESP_EVENT_ANY_ID, wifi_event, NULL));
    ESP_ERROR_CHECK(esp_event_handler_register(IP_EVENT, IP_EVENT_STA_GOT_IP, wifi_event, NULL));
    wifi_config_t config = {0};
    memcpy(config.sta.ssid, ssid, strlen(ssid));
    memcpy(config.sta.password, password, strlen(password));
    config.sta.threshold.authmode = password[0] ? WIFI_AUTH_WPA2_PSK : WIFI_AUTH_OPEN;
    ESP_ERROR_CHECK(esp_wifi_set_mode(WIFI_MODE_STA));
    ESP_ERROR_CHECK(esp_wifi_set_config(WIFI_IF_STA, &config));
    ESP_ERROR_CHECK(esp_wifi_start());
    ESP_ERROR_CHECK(esp_wifi_connect());

    EventBits_t bits = xEventGroupWaitBits(
        s_wifi_events, WIFI_CONNECTED, pdFALSE, pdTRUE, pdMS_TO_TICKS(30000));
    return (bits & WIFI_CONNECTED) ? ESP_OK : ESP_ERR_TIMEOUT;
}

void passport_wifi_clear_config(void)
{
    nvs_handle_t nvs;
    if (nvs_open(CONFIG_NAMESPACE, NVS_READWRITE, &nvs) == ESP_OK) {
        nvs_erase_all(nvs);
        nvs_commit(nvs);
        nvs_close(nvs);
    }
}

int passport_wifi_rssi(void)
{
    wifi_ap_record_t access_point = {0};
    return esp_wifi_sta_get_ap_info(&access_point) == ESP_OK ? access_point.rssi : 0;
}
