#include "shadow/game_api.h"
#include "components.h"

#include <algorithm>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <fstream>
#include <sstream>
#include <string>
#include <vector>

static constexpr uint32_t COMP_TRANSFORM         = 0;
static constexpr uint32_t COMP_PLAYER_CONTROLLER = 1;
static constexpr uint32_t COMP_HEALTH            = 2;
static constexpr uint32_t COMP_COUNT             = 3;

static ComponentMeta COMP_META[COMP_COUNT] = {
    {"Transform",        sizeof(Transform),        alignof(Transform)},
    {"PlayerController", sizeof(PlayerController), alignof(PlayerController)},
    {"Health",           sizeof(Health),           alignof(Health)},
};

struct RuntimeEntity {
    EntityId    id;
    std::string scene_id;
    std::string name;
    void*       components[COMP_COUNT] = {nullptr, nullptr, nullptr};
};

static std::vector<RuntimeEntity> g_entities;
static std::vector<EntityId>      g_entity_ids;
static uint64_t                   g_next_entity_id = 1;
static ShadowEngineCtx*           g_context        = nullptr;

static void refresh_entity_cache() {
    g_entity_ids.clear();
    g_entity_ids.reserve(g_entities.size());
    for (const auto& entity : g_entities) {
        g_entity_ids.push_back(entity.id);
    }
}

static RuntimeEntity* find_entity(EntityId id) {
    for (auto& entity : g_entities) {
        if (entity.id == id) return &entity;
    }
    return nullptr;
}

static RuntimeEntity* find_entity_by_scene_id(const char* scene_id) {
    if (scene_id == nullptr || scene_id[0] == '\0') return nullptr;
    for (auto& entity : g_entities) {
        if (entity.scene_id == scene_id) return &entity;
    }
    return nullptr;
}

static void attach_component(RuntimeEntity& entity, uint32_t type_id) {
    if (type_id >= COMP_COUNT || entity.components[type_id] != nullptr) return;

    const size_t size = COMP_META[type_id].size;
    void* memory = ::operator new(size);
    std::memset(memory, 0, size);

    switch (type_id) {
        case COMP_TRANSFORM:         new (memory) Transform{};         break;
        case COMP_PLAYER_CONTROLLER: new (memory) PlayerController{};  break;
        case COMP_HEALTH:            new (memory) Health{};            break;
    }

    entity.components[type_id] = memory;
}

static void detach_component(RuntimeEntity& entity, uint32_t type_id) {
    if (type_id >= COMP_COUNT || entity.components[type_id] == nullptr) return;
    ::operator delete(entity.components[type_id]);
    entity.components[type_id] = nullptr;
}

static void detach_all_components(RuntimeEntity& entity) {
    for (uint32_t index = 0; index < COMP_COUNT; ++index) {
        if (!entity.components[index]) continue;
        ::operator delete(entity.components[index]);
        entity.components[index] = nullptr;
    }
}

extern "C" void shadow_init(ShadowEngineCtx* ctx) {
    g_context = ctx;

    if (g_entities.empty()) {
        RuntimeEntity player;
        player.id   = g_next_entity_id++;
        player.scene_id = "player";
        player.name = "Player";
        attach_component(player, COMP_TRANSFORM);
        attach_component(player, COMP_PLAYER_CONTROLLER);
        attach_component(player, COMP_HEALTH);
        auto* transform = static_cast<Transform*>(player.components[COMP_TRANSFORM]);
        transform->position[1] = 1.0f;
        g_entities.push_back(std::move(player));

        RuntimeEntity ground;
        ground.id   = g_next_entity_id++;
        ground.scene_id = "ground";
        ground.name = "Ground";
        attach_component(ground, COMP_TRANSFORM);
        g_entities.push_back(std::move(ground));
    }

    refresh_entity_cache();
}

extern "C" void shadow_update(float delta_time) {
    if (g_context != nullptr && delta_time >= 0.0f) {
        g_context->frame_index += 1;
    }
}

extern "C" void shadow_shutdown(void) {
    for (auto& entity : g_entities) {
        detach_all_components(entity);
    }
    g_context = nullptr;
}

extern "C" uint32_t shadow_component_count(void) {
    return COMP_COUNT;
}

extern "C" ComponentMeta* shadow_component_meta(uint32_t index) {
    if (index >= COMP_COUNT) return nullptr;
    return &COMP_META[index];
}

extern "C" void* shadow_get_component(EntityId id, uint32_t component_type) {
    if (component_type >= COMP_COUNT) return nullptr;
    auto* entity = find_entity(id);
    return entity ? entity->components[component_type] : nullptr;
}

extern "C" void shadow_set_component(EntityId id, uint32_t component_type, void* data) {
    if (component_type >= COMP_COUNT || data == nullptr) return;
    auto* entity = find_entity(id);
    if (!entity) return;
    if (!entity->components[component_type]) {
        attach_component(*entity, component_type);
    }
    std::memcpy(entity->components[component_type], data, COMP_META[component_type].size);
}

extern "C" void shadow_remove_component(EntityId id, uint32_t component_type) {
    if (component_type >= COMP_COUNT) return;
    auto* entity = find_entity(id);
    if (!entity) return;
    detach_component(*entity, component_type);
}

extern "C" EntityId shadow_create_entity(const char* name) {
    RuntimeEntity entity;
    entity.id = g_next_entity_id++;
    entity.scene_id.clear();
    entity.name = (name != nullptr) ? name : "Entity";
    attach_component(entity, COMP_TRANSFORM);
    g_entities.push_back(std::move(entity));
    refresh_entity_cache();
    return g_entities.back().id;
}

extern "C" void shadow_destroy_entity(EntityId id) {
    for (auto it = g_entities.begin(); it != g_entities.end(); ++it) {
        if (it->id == id) {
            detach_all_components(*it);
            g_entities.erase(it);
            break;
        }
    }
    refresh_entity_cache();
}

extern "C" void shadow_set_entity_name(EntityId id, const char* name) {
    auto* entity = find_entity(id);
    if (!entity || name == nullptr) return;
    entity->name = name;
}

extern "C" void shadow_set_entity_scene_id(EntityId id, const char* scene_id) {
    auto* entity = find_entity(id);
    if (!entity || scene_id == nullptr) return;
    entity->scene_id = scene_id;
}

extern "C" EntityId shadow_find_entity_by_scene_id(const char* scene_id) {
    auto* entity = find_entity_by_scene_id(scene_id);
    return entity ? entity->id : 0;
}

extern "C" uint32_t shadow_entity_count(void) {
    return static_cast<uint32_t>(g_entities.size());
}

extern "C" EntityId* shadow_entity_list(void) {
    refresh_entity_cache();
    return g_entity_ids.empty() ? nullptr : g_entity_ids.data();
}

static std::string fmt_f(float value) {
    char buffer[32];
    std::snprintf(buffer, sizeof(buffer), "%.6g", value);
    return buffer;
}

static std::string fmt_farr(const float* values, int count) {
    std::string out = "[";
    for (int index = 0; index < count; ++index) {
        if (index > 0) out += ", ";
        out += fmt_f(values[index]);
    }
    out += "]";
    return out;
}

extern "C" void shadow_save_scene(const char* path) {
    if (path == nullptr) return;

    std::string out;
    out += "[scene]\n";
    out += "name = \"MainLevel\"\n";
    out += "version = \"1.0\"\n";
    out += "runtime = \"cpp23\"\n";

    for (const auto& entity : g_entities) {
        out += "\n[[entity]]\n";
        out += "id = \"" + (entity.scene_id.empty() ? std::to_string(entity.id) : entity.scene_id) + "\"\n";
        out += "name = \"" + entity.name + "\"\n";

        auto* transform = static_cast<Transform*>(entity.components[COMP_TRANSFORM]);
        if (transform) {
            out += "\n  [[entity.component]]\n";
            out += "  type = \"Transform\"\n";
            out += "  position = " + fmt_farr(transform->position, 3) + "\n";
            out += "  rotation = " + fmt_farr(transform->rotation, 4) + "\n";
            out += "  scale = " + fmt_farr(transform->scale, 3) + "\n";
        }

        auto* player_controller = static_cast<PlayerController*>(entity.components[COMP_PLAYER_CONTROLLER]);
        if (player_controller) {
            out += "\n  [[entity.component]]\n";
            out += "  type = \"PlayerController\"\n";
            out += "  speed = " + fmt_f(player_controller->speed) + "\n";
            out += "  jump_force = " + fmt_f(player_controller->jump_force) + "\n";
        }

        auto* health = static_cast<Health*>(entity.components[COMP_HEALTH]);
        if (health) {
            out += "\n  [[entity.component]]\n";
            out += "  type = \"Health\"\n";
            out += "  current = " + fmt_f(health->current) + "\n";
            out += "  max = " + fmt_f(health->max) + "\n";
            out += "  regen = ";
            out += health->regen ? "true" : "false";
            out += "\n";
        }
    }

    std::ofstream file(path);
    if (file.is_open()) {
        file << out;
    }
}

extern "C" void shadow_load_scene(const char* path) {
    if (path == nullptr) return;

    std::ifstream file(path);
    if (!file.is_open()) return;

    for (auto& entity : g_entities) {
        detach_all_components(entity);
    }
    g_entities.clear();
    g_next_entity_id = 1;

    enum class Section { None, Scene, Entity, Component };

    Section section = Section::None;
    RuntimeEntity* current_entity = nullptr;
    uint32_t current_component_type = COMP_COUNT;

    auto trim = [](const std::string& value) -> std::string {
        const size_t start = value.find_first_not_of(" \t\r\n");
        if (start == std::string::npos) return "";
        const size_t end = value.find_last_not_of(" \t\r\n");
        return value.substr(start, end - start + 1);
    };

    auto strip_quotes = [](const std::string& value) -> std::string {
        if (value.size() >= 2 && value.front() == '"' && value.back() == '"') {
            return value.substr(1, value.size() - 2);
        }
        return value;
    };

    auto parse_farr = [&](const std::string& value, float* out, int count) {
        const size_t open = value.find('[');
        const size_t close = value.find(']');
        if (open == std::string::npos || close == std::string::npos) return;
        std::istringstream stream(value.substr(open + 1, close - open - 1));
        std::string token;
        int index = 0;
        while (std::getline(stream, token, ',') && index < count) {
            token = strip_quotes(trim(token));
            if (!token.empty()) out[index++] = std::stof(token);
        }
    };

    std::string line;
    while (std::getline(file, line)) {
        std::string trimmed = trim(line);
        if (trimmed.empty() || trimmed[0] == '#') continue;

        if (trimmed == "[scene]") {
            section = Section::Scene;
            continue;
        }
        if (trimmed == "[[entity]]") {
            g_entities.emplace_back();
            current_entity = &g_entities.back();
            current_entity->id = g_next_entity_id++;
            current_component_type = COMP_COUNT;
            section = Section::Entity;
            continue;
        }
        if (trimmed == "[[entity.component]]") {
            current_component_type = COMP_COUNT;
            section = Section::Component;
            continue;
        }

        const size_t eq = trimmed.find('=');
        if (eq == std::string::npos) continue;
        const std::string key = trim(trimmed.substr(0, eq));
        const std::string value = trim(trimmed.substr(eq + 1));

        if (section == Section::Entity && current_entity != nullptr) {
            if (key == "id") current_entity->scene_id = strip_quotes(value);
            else if (key == "name") current_entity->name = strip_quotes(value);
            continue;
        }

        if (section != Section::Component || current_entity == nullptr) {
            continue;
        }

        if (key == "type") {
            const std::string type_name = strip_quotes(value);
            if (type_name == "Transform") {
                current_component_type = COMP_TRANSFORM;
                attach_component(*current_entity, COMP_TRANSFORM);
            } else if (type_name == "PlayerController") {
                current_component_type = COMP_PLAYER_CONTROLLER;
                attach_component(*current_entity, COMP_PLAYER_CONTROLLER);
            } else if (type_name == "Health") {
                current_component_type = COMP_HEALTH;
                attach_component(*current_entity, COMP_HEALTH);
            } else {
                current_component_type = COMP_COUNT;
            }
            continue;
        }

        if (current_component_type == COMP_TRANSFORM) {
            auto* transform = static_cast<Transform*>(current_entity->components[COMP_TRANSFORM]);
            if (transform == nullptr) continue;
            if (key == "position") parse_farr(value, transform->position, 3);
            else if (key == "rotation") parse_farr(value, transform->rotation, 4);
            else if (key == "scale") parse_farr(value, transform->scale, 3);
        } else if (current_component_type == COMP_PLAYER_CONTROLLER) {
            auto* player_controller = static_cast<PlayerController*>(current_entity->components[COMP_PLAYER_CONTROLLER]);
            if (player_controller == nullptr) continue;
            if (key == "speed") player_controller->speed = std::stof(value);
            else if (key == "jump_force") player_controller->jump_force = std::stof(value);
        } else if (current_component_type == COMP_HEALTH) {
            auto* health = static_cast<Health*>(current_entity->components[COMP_HEALTH]);
            if (health == nullptr) continue;
            if (key == "current") health->current = std::stof(value);
            else if (key == "max") health->max = std::stof(value);
            else if (key == "regen") health->regen = (value == "true");
        }
    }

    refresh_entity_cache();
}
