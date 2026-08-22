#include "passport_wifi.h"

#include "passport_policy.h"
#include "passport_storage.h"
#include "passport_ui.h"
#include "esp_event.h"
#include "esp_http_server.h"
#include "esp_log.h"
#include "esp_mac.h"
#include "esp_netif.h"
#include "esp_system.h"
#include "esp_wifi.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"

#include <ctype.h>
#include <stdio.h>
#include <string.h>

static const char *TAG = "passport_wifi";
static char s_captive_portal_uri[] = "http://192.168.4.1/";
static volatile bool s_connected;

static void wifi_event(void *arg, esp_event_base_t base, int32_t id, void *data)
{
    (void)arg;
    (void)data;
    if (base == WIFI_EVENT && id == WIFI_EVENT_STA_DISCONNECTED) {
        s_connected = false;
        passport_ui_set(PASSPORT_UI_CONNECTING, "Reconnecting Wi-Fi");
        esp_err_t err = esp_wifi_connect();
        if (err != ESP_OK) ESP_LOGW(TAG, "Wi-Fi reconnect scheduling failed: %s", esp_err_to_name(err));
    } else if (base == IP_EVENT && id == IP_EVENT_STA_GOT_IP) {
        s_connected = true;
        passport_ui_set(PASSPORT_UI_CONNECTING, "Connecting to Ripple");
        ESP_LOGI(TAG, "Wi-Fi connected");
    }
}

static int hex_value(char character)
{
    if (character >= '0' && character <= '9') return character - '0';
    character = (char)tolower((unsigned char)character);
    if (character >= 'a' && character <= 'f') return character - 'a' + 10;
    return -1;
}

static bool url_decode(char *output, size_t output_size, const char *input, size_t input_len)
{
    size_t written = 0;
    for (size_t i = 0; i < input_len; ++i) {
        if (written + 1 >= output_size) return false;
        unsigned char decoded = (unsigned char)input[i];
        if (input[i] == '+') {
            decoded = ' ';
        } else if (input[i] == '%') {
            if (i + 2 >= input_len) return false;
            int hi = hex_value(input[i + 1]);
            int lo = hex_value(input[i + 2]);
            if (hi < 0 || lo < 0) return false;
            decoded = (unsigned char)((hi << 4) | lo);
            i += 2;
        }
        if (decoded == '\0') return false;
        output[written++] = (char)decoded;
    }
    output[written] = '\0';
    return true;
}

static bool form_value(const char *body, const char *name, char *value, size_t value_size)
{
    size_t name_len = strlen(name);
    const char *cursor = body;
    while (cursor && *cursor) {
        if (strncmp(cursor, name, name_len) == 0 && cursor[name_len] == '=') {
            const char *start = cursor + name_len + 1;
            const char *end = strchr(start, '&');
            return url_decode(value, value_size, start,
                              end ? (size_t)(end - start) : strlen(start));
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
        "<input name=ssid maxlength=32 placeholder='Wi-Fi name' required>"
        "<input name=password maxlength=64 type=password placeholder='Wi-Fi password'>"
        "<input name=gateway maxlength=95 value='140.143.229.103:8700' required>"
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

    passport_wifi_config_t config = {0};
    if (!form_value(body, "ssid", config.ssid, sizeof(config.ssid)) ||
        !form_value(body, "gateway", config.gateway, sizeof(config.gateway)) ||
        !form_value(body, "password", config.password, sizeof(config.password)) ||
        config.ssid[0] == '\0' || strlen(config.password) > 63 ||
        !passport_gateway_is_valid(config.gateway)) {
        return httpd_resp_send_err(request, HTTPD_400_BAD_REQUEST,
                                   "Invalid SSID, password, or gateway host:port");
    }

    esp_err_t err = passport_storage_save_wifi(&config);
    if (err != ESP_OK) {
        ESP_LOGE(TAG, "configuration save failed: %s", esp_err_to_name(err));
        return httpd_resp_send_err(request, HTTPD_500_INTERNAL_SERVER_ERROR,
                                   "Could not save configuration");
    }

    passport_ui_set(PASSPORT_UI_CONNECTING, "Saved. Restarting...");
    httpd_resp_sendstr(request, "Saved. The Passport is restarting.");
    if (xTaskCreate(delayed_restart, "restart", 2048, NULL, 4, NULL) != pdPASS) {
        ESP_LOGE(TAG, "restart task creation failed; restart the device manually");
    }
    return ESP_OK;
}

static esp_err_t start_provisioning(void)
{
    uint8_t mac[6];
    esp_err_t err = esp_read_mac(mac, ESP_MAC_WIFI_SOFTAP);
    if (err != ESP_OK) return err;
    char ap_name[33];
    snprintf(ap_name, sizeof(ap_name), "Ripple-Passport-%02X%02X", mac[4], mac[5]);

    esp_netif_t *ap_netif = esp_netif_create_default_wifi_ap();
    if (!ap_netif) return ESP_ERR_NO_MEM;
    err = esp_netif_dhcps_stop(ap_netif);
    if (err != ESP_OK && err != ESP_ERR_ESP_NETIF_DHCP_ALREADY_STOPPED) return err;
    err = esp_netif_dhcps_option(ap_netif, ESP_NETIF_OP_SET,
                                 ESP_NETIF_CAPTIVEPORTAL_URI,
                                 s_captive_portal_uri, sizeof(s_captive_portal_uri));
    if (err != ESP_OK) return err;
    if ((err = esp_netif_dhcps_start(ap_netif)) != ESP_OK) return err;
    wifi_init_config_t init = WIFI_INIT_CONFIG_DEFAULT();
    if ((err = esp_wifi_init(&init)) != ESP_OK) return err;
    wifi_config_t config = {0};
    config.ap.ssid_len = strlen(ap_name);
    memcpy(config.ap.ssid, ap_name, config.ap.ssid_len);
    config.ap.channel = 1;
    config.ap.max_connection = 4;
    config.ap.authmode = WIFI_AUTH_OPEN;
    if ((err = esp_wifi_set_mode(WIFI_MODE_AP)) != ESP_OK ||
        (err = esp_wifi_set_config(WIFI_IF_AP, &config)) != ESP_OK ||
        (err = esp_wifi_start()) != ESP_OK) {
        return err;
    }

    httpd_config_t server_config = HTTPD_DEFAULT_CONFIG();
    httpd_handle_t server = NULL;
    if ((err = httpd_start(&server, &server_config)) != ESP_OK) return err;
    const httpd_uri_t root = {.uri = "/", .method = HTTP_GET, .handler = setup_page};
    const httpd_uri_t configure = {
        .uri = "/configure", .method = HTTP_POST, .handler = save_config};
    if ((err = httpd_register_uri_handler(server, &root)) != ESP_OK ||
        (err = httpd_register_uri_handler(server, &configure)) != ESP_OK) {
        httpd_stop(server);
        return err;
    }
    static const char *portal_paths[] = {
        "/generate_204", "/hotspot-detect.html", "/ncsi.txt", "/connecttest.txt",
    };
    for (size_t i = 0; i < sizeof(portal_paths) / sizeof(portal_paths[0]); ++i) {
        const httpd_uri_t portal = {
            .uri = portal_paths[i], .method = HTTP_GET, .handler = setup_page};
        if ((err = httpd_register_uri_handler(server, &portal)) != ESP_OK) {
            httpd_stop(server);
            return err;
        }
    }

    char detail[80];
    snprintf(detail, sizeof(detail), "%s\nOpen 192.168.4.1", ap_name);
    passport_ui_set(PASSPORT_UI_SETUP, detail);
    ESP_LOGI(TAG, "provisioning AP %s", ap_name);
    return ESP_ERR_NOT_FINISHED;
}

esp_err_t passport_wifi_start(char *gateway, size_t gateway_size)
{
    if (!gateway || gateway_size == 0) return ESP_ERR_INVALID_ARG;
    esp_err_t err = esp_netif_init();
    if (err != ESP_OK) return err;
    err = esp_event_loop_create_default();
    if (err != ESP_OK) return err;

    passport_wifi_config_t stored = {0};
    if (passport_storage_load_wifi(&stored) != ESP_OK ||
        strlen(stored.password) > 63 || !passport_gateway_is_valid(stored.gateway)) {
        return start_provisioning();
    }
    if (strlen(stored.gateway) + 1 > gateway_size) return ESP_ERR_INVALID_SIZE;
    memcpy(gateway, stored.gateway, strlen(stored.gateway) + 1);

    passport_ui_set(PASSPORT_UI_CONNECTING, "Connecting Wi-Fi");
    if (!esp_netif_create_default_wifi_sta()) return ESP_ERR_NO_MEM;
    wifi_init_config_t init = WIFI_INIT_CONFIG_DEFAULT();
    if ((err = esp_wifi_init(&init)) != ESP_OK) return err;
    if ((err = esp_event_handler_register(WIFI_EVENT, ESP_EVENT_ANY_ID, wifi_event, NULL)) != ESP_OK ||
        (err = esp_event_handler_register(IP_EVENT, IP_EVENT_STA_GOT_IP, wifi_event, NULL)) != ESP_OK) {
        return err;
    }
    wifi_config_t config = {0};
    memcpy(config.sta.ssid, stored.ssid, strlen(stored.ssid));
    memcpy(config.sta.password, stored.password, strlen(stored.password));
    config.sta.threshold.authmode = stored.password[0] ? WIFI_AUTH_WPA2_PSK : WIFI_AUTH_OPEN;
    if ((err = esp_wifi_set_mode(WIFI_MODE_STA)) != ESP_OK ||
        (err = esp_wifi_set_config(WIFI_IF_STA, &config)) != ESP_OK ||
        (err = esp_wifi_start()) != ESP_OK ||
        (err = esp_wifi_connect()) != ESP_OK) {
        return err;
    }

    // Realtime starts immediately and lets its WebSocket reconnect loop follow Wi-Fi.
    // This avoids a one-shot boot timeout that could never recover after app_main returned.
    return ESP_OK;
}

void passport_wifi_clear_config(void)
{
    esp_err_t err = passport_storage_clear_wifi();
    if (err != ESP_OK) ESP_LOGW(TAG, "Wi-Fi clear failed: %s", esp_err_to_name(err));
}

int passport_wifi_rssi(void)
{
    wifi_ap_record_t access_point = {0};
    return esp_wifi_sta_get_ap_info(&access_point) == ESP_OK ? access_point.rssi : 0;
}

bool passport_wifi_is_connected(void)
{
    return s_connected;
}
