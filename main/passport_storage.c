#include "passport_storage.h"

#include "esp_log.h"
#include "nvs.h"
#include "nvs_flash.h"

#include <stdbool.h>
#include <string.h>

#define BACKUP_PARTITION "ripple_backup"
#define WIFI_NAMESPACE "ripple"
#define UI_NAMESPACE "ripple_ui"

static const char *TAG = "passport_store";
static bool s_primary_ready;
static bool s_backup_ready;

static esp_err_t initialize_partition(const char *partition, bool primary)
{
    esp_err_t err = primary ? nvs_flash_init() : nvs_flash_init_partition(partition);
    if (err != ESP_ERR_NVS_NO_FREE_PAGES && err != ESP_ERR_NVS_NEW_VERSION_FOUND) {
        return err;
    }

    ESP_LOGW(TAG, "%s NVS requires recovery: %s",
             primary ? "primary" : "backup", esp_err_to_name(err));
    err = primary ? nvs_flash_erase() : nvs_flash_erase_partition(partition);
    if (err != ESP_OK) return err;
    return primary ? nvs_flash_init() : nvs_flash_init_partition(partition);
}

static esp_err_t open_namespace(bool backup, const char *name, nvs_open_mode_t mode,
                                nvs_handle_t *handle)
{
    if (backup) {
        if (!s_backup_ready) return ESP_ERR_INVALID_STATE;
        return nvs_open_from_partition(BACKUP_PARTITION, name, mode, handle);
    }
    if (!s_primary_ready) return ESP_ERR_INVALID_STATE;
    return nvs_open(name, mode, handle);
}

static esp_err_t write_volume(bool backup, uint8_t volume)
{
    nvs_handle_t nvs;
    esp_err_t err = open_namespace(backup, UI_NAMESPACE, NVS_READWRITE, &nvs);
    if (err != ESP_OK) return err;
    err = nvs_set_u8(nvs, "volume", volume);
    if (err == ESP_OK) err = nvs_commit(nvs);
    nvs_close(nvs);
    return err;
}

static esp_err_t read_volume(bool backup, uint8_t *volume)
{
    nvs_handle_t nvs;
    esp_err_t err = open_namespace(backup, UI_NAMESPACE, NVS_READONLY, &nvs);
    if (err != ESP_OK) return err;
    err = nvs_get_u8(nvs, "volume", volume);
    nvs_close(nvs);
    return err;
}

static esp_err_t write_wifi(bool backup, const passport_wifi_config_t *config)
{
    nvs_handle_t nvs;
    esp_err_t err = open_namespace(backup, WIFI_NAMESPACE, NVS_READWRITE, &nvs);
    if (err != ESP_OK) return err;
    if ((err = nvs_set_str(nvs, "ssid", config->ssid)) == ESP_OK &&
        (err = nvs_set_str(nvs, "password", config->password)) == ESP_OK &&
        (err = nvs_set_str(nvs, "gateway", config->gateway)) == ESP_OK) {
        err = nvs_commit(nvs);
    }
    nvs_close(nvs);
    return err;
}

static esp_err_t read_string(nvs_handle_t nvs, const char *key, char *value, size_t size,
                             bool allow_empty)
{
    size_t needed = size;
    esp_err_t err = nvs_get_str(nvs, key, value, &needed);
    if (err != ESP_OK) return err;
    return allow_empty || value[0] != '\0' ? ESP_OK : ESP_ERR_NOT_FOUND;
}

static esp_err_t read_wifi(bool backup, passport_wifi_config_t *config)
{
    nvs_handle_t nvs;
    esp_err_t err = open_namespace(backup, WIFI_NAMESPACE, NVS_READONLY, &nvs);
    if (err != ESP_OK) return err;
    memset(config, 0, sizeof(*config));
    if ((err = read_string(nvs, "ssid", config->ssid, sizeof(config->ssid), false)) == ESP_OK &&
        (err = read_string(nvs, "password", config->password, sizeof(config->password), true)) == ESP_OK) {
        err = read_string(nvs, "gateway", config->gateway, sizeof(config->gateway), false);
    }
    nvs_close(nvs);
    return err;
}

static esp_err_t erase_wifi(bool backup)
{
    nvs_handle_t nvs;
    esp_err_t err = open_namespace(backup, WIFI_NAMESPACE, NVS_READWRITE, &nvs);
    if (err != ESP_OK) return err;
    err = nvs_erase_all(nvs);
    if (err == ESP_OK) err = nvs_commit(nvs);
    nvs_close(nvs);
    return err;
}

esp_err_t passport_storage_init(void)
{
    esp_err_t primary = initialize_partition(NULL, true);
    s_primary_ready = primary == ESP_OK;
    esp_err_t backup = initialize_partition(BACKUP_PARTITION, false);
    s_backup_ready = backup == ESP_OK;

    if (!s_backup_ready) {
        ESP_LOGW(TAG, "backup NVS unavailable: %s", esp_err_to_name(backup));
    }
    return primary;
}

uint8_t passport_storage_load_volume(uint8_t fallback)
{
    uint8_t volume = fallback;
    if (read_volume(false, &volume) == ESP_OK && volume <= 100) {
        esp_err_t err = write_volume(true, volume);
        if (err != ESP_OK) {
            ESP_LOGW(TAG, "volume mirror failed: %s", esp_err_to_name(err));
        }
        return volume;
    }
    if (read_volume(true, &volume) == ESP_OK && volume <= 100) {
        write_volume(false, volume);
        ESP_LOGI(TAG, "restored volume from backup NVS");
        return volume;
    }
    return fallback;
}

esp_err_t passport_storage_save_volume(uint8_t volume)
{
    esp_err_t primary = write_volume(false, volume);
    esp_err_t backup = write_volume(true, volume);
    if (backup != ESP_OK) ESP_LOGW(TAG, "volume backup failed: %s", esp_err_to_name(backup));
    return primary;
}

esp_err_t passport_storage_load_wifi(passport_wifi_config_t *config)
{
    if (!config) return ESP_ERR_INVALID_ARG;
    esp_err_t err = read_wifi(false, config);
    if (err == ESP_OK) {
        esp_err_t backup = write_wifi(true, config);
        if (backup != ESP_OK) {
            ESP_LOGW(TAG, "Wi-Fi mirror failed: %s", esp_err_to_name(backup));
        }
        return ESP_OK;
    }
    err = read_wifi(true, config);
    if (err == ESP_OK) {
        write_wifi(false, config);
        ESP_LOGI(TAG, "restored Wi-Fi configuration from backup NVS");
    }
    return err;
}

esp_err_t passport_storage_save_wifi(const passport_wifi_config_t *config)
{
    if (!config) return ESP_ERR_INVALID_ARG;
    esp_err_t primary = write_wifi(false, config);
    esp_err_t backup = write_wifi(true, config);
    if (backup != ESP_OK) ESP_LOGW(TAG, "Wi-Fi backup failed: %s", esp_err_to_name(backup));
    return primary;
}

esp_err_t passport_storage_clear_wifi(void)
{
    esp_err_t primary = erase_wifi(false);
    esp_err_t backup = erase_wifi(true);
    if (backup != ESP_OK) ESP_LOGW(TAG, "Wi-Fi backup clear failed: %s", esp_err_to_name(backup));
    return primary;
}
