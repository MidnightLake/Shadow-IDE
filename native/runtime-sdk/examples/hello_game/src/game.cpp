#include "shadow/game_api.h"
#include "components.h"

#include <algorithm>
#include <cstdint>
#include <cstring>
#include <fstream>
#include <sstream>
#include <string>
#include <vector>

// Component type IDs — stable across reloads (FNV-1a hashes would be better,
// but sequential ints are fine for the example).
static constexpr uint32_t COMP_TRANSFORM         = 0;
static constexpr uint32_t COMP_PLAYER_CONTROLLER = 1;
static constexpr uint32_t COMP_HEALTH            = 2;
static constexpr uint32_t COMP_COUNT             = 3;

// ─── Reflection metadata (static, survives hot-reload) ───────────────────────

static const char* COMP_NAMES[COMP_COUNT] = {
    "Transform",
    "PlayerController",
    "Health",
};

static ComponentMeta COMP_META[COMP_COUNT] = {
    {"Transform",         sizeof(TransformData),         alignof(TransformData)},
    {"PlayerController",  sizeof(PlayerControllerData),  alignof(PlayerControllerData)},
    {"Health",            sizeof(HealthData),            alignof(HealthData)},
};

// ─── Entity storage ───────────────────────────────────────────────────────────

struct RuntimeEntity {
    EntityId    id;
    std::string scene_id;
    std::string name;
    // One slot per component type; nullptr = not attached
    void*       components[COMP_COUNT] = {nullptr, nullptr, nullptr};
};

static std::vector<RuntimeEntity> g_entities;
static std::vector<EntityId>      g_entity_ids;
static uint64_t                   g_next_entity_id = 1;
static ShadowEngineCtx*           g_context        = nullptr;

static void refresh_entity_cache() {
    g_entity_ids.clear();
    g_entity_ids.reserve(g_entities.size());
    for (const auto& e : g_entities) {
        g_entity_ids.push_back(e.id);
    }
}

static RuntimeEntity* find_entity(EntityId id) {
    for (auto& e : g_entities) {
        if (e.id == id) return &e;
    }
    return nullptr;
}

static RuntimeEntity* find_entity_by_scene_id(const char* scene_id) {
    if (scene_id == nullptr || scene_id[0] == '\0') return nullptr;
    for (auto& e : g_entities) {
        if (e.scene_id == scene_id) return &e;
    }
    return nullptr;
}

// Attach a zeroed component to an entity if not already attached.
static void attach_component(RuntimeEntity& entity, uint32_t type_id) {
    if (type_id >= COMP_COUNT) return;
    if (entity.components[type_id] != nullptr) return;

    size_t sz = COMP_META[type_id].size;
    void* mem = ::operator new(sz);
    std::memset(mem, 0, sz);

    // Initialise with defaults via placement new
    switch (type_id) {
        case COMP_TRANSFORM:         new (mem) TransformData{};         break;
        case COMP_PLAYER_CONTROLLER: new (mem) PlayerControllerData{};  break;
        case COMP_HEALTH:            new (mem) HealthData{};            break;
    }

    entity.components[type_id] = mem;
}

static void detach_all_components(RuntimeEntity& entity) {
    for (uint32_t i = 0; i < COMP_COUNT; ++i) {
        if (entity.components[i]) {
            ::operator delete(entity.components[i]);
            entity.components[i] = nullptr;
        }
    }
}

static void detach_component(RuntimeEntity& entity, uint32_t type_id) {
    if (type_id >= COMP_COUNT || entity.components[type_id] == nullptr) return;
    ::operator delete(entity.components[type_id]);
    entity.components[type_id] = nullptr;
}

// ─── C ABI implementation ─────────────────────────────────────────────────────

extern "C" void shadow_init(ShadowEngineCtx* ctx) {
    g_context = ctx;

    if (g_entities.empty()) {
        // Player entity
        {
            RuntimeEntity player;
            player.id   = g_next_entity_id++;
            player.scene_id = "player";
            player.name = "Player";
            attach_component(player, COMP_TRANSFORM);
            attach_component(player, COMP_PLAYER_CONTROLLER);
            attach_component(player, COMP_HEALTH);
            // Set non-default position
            auto* t      = static_cast<TransformData*>(player.components[COMP_TRANSFORM]);
            t->position[1] = 1.0f; // Y = 1 m above ground
            g_entities.push_back(std::move(player));
        }

        // Directional light
        {
            RuntimeEntity light;
            light.id   = g_next_entity_id++;
            light.scene_id = "directional_light";
            light.name = "DirectionalLight";
            attach_component(light, COMP_TRANSFORM);
            auto* t      = static_cast<TransformData*>(light.components[COMP_TRANSFORM]);
            t->rotation[0] = -0.52f; t->rotation[3] = 0.86f; // ~30° tilt
            g_entities.push_back(std::move(light));
        }

        // Ground plane
        {
            RuntimeEntity ground;
            ground.id   = g_next_entity_id++;
            ground.scene_id = "ground";
            ground.name = "Ground";
            attach_component(ground, COMP_TRANSFORM);
            g_entities.push_back(std::move(ground));
        }
    }

    refresh_entity_cache();
}

extern "C" void shadow_update(float delta_time) {
    if (g_context != nullptr && delta_time >= 0.0F) {
        g_context->frame_index += 1;
    }
    // Game systems would tick here: physics, animations, scripts…
}

extern "C" void shadow_shutdown(void) {
    for (auto& e : g_entities) {
        detach_all_components(e);
    }
    g_context = nullptr;
    // NOTE: entity list is preserved across hot-reload so inspected
    // entity IDs remain stable. Systems reset; data survives.
}

// ─── Reflection ───────────────────────────────────────────────────────────────

extern "C" uint32_t shadow_component_count(void) {
    return COMP_COUNT;
}

extern "C" ComponentMeta* shadow_component_meta(uint32_t index) {
    if (index >= COMP_COUNT) return nullptr;
    return &COMP_META[index];
}

extern "C" void* shadow_get_component(EntityId id, uint32_t component_type) {
    if (component_type >= COMP_COUNT) return nullptr;
    auto* e = find_entity(id);
    return e ? e->components[component_type] : nullptr;
}

extern "C" void shadow_set_component(EntityId id, uint32_t component_type, void* data) {
    if (component_type >= COMP_COUNT || data == nullptr) return;
    auto* e = find_entity(id);
    if (!e) return;
    if (!e->components[component_type]) {
        attach_component(*e, component_type);
    }
    std::memcpy(e->components[component_type], data, COMP_META[component_type].size);
}

extern "C" void shadow_remove_component(EntityId id, uint32_t component_type) {
    if (component_type >= COMP_COUNT) return;
    auto* e = find_entity(id);
    if (!e) return;
    detach_component(*e, component_type);
}

// ─── Entity management ────────────────────────────────────────────────────────

extern "C" EntityId shadow_create_entity(const char* name) {
    RuntimeEntity e;
    e.id   = g_next_entity_id++;
    e.scene_id.clear();
    e.name = (name != nullptr) ? name : "Entity";
    attach_component(e, COMP_TRANSFORM); // every entity gets a transform
    g_entities.push_back(std::move(e));
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
    auto* e = find_entity(id);
    if (!e || name == nullptr) return;
    e->name = name;
}

extern "C" void shadow_set_entity_scene_id(EntityId id, const char* scene_id) {
    auto* e = find_entity(id);
    if (!e || scene_id == nullptr) return;
    e->scene_id = scene_id;
}

extern "C" EntityId shadow_find_entity_by_scene_id(const char* scene_id) {
    auto* e = find_entity_by_scene_id(scene_id);
    return e ? e->id : 0;
}

extern "C" uint32_t shadow_entity_count(void) {
    return static_cast<uint32_t>(g_entities.size());
}

extern "C" EntityId* shadow_entity_list(void) {
    refresh_entity_cache();
    return g_entity_ids.empty() ? nullptr : g_entity_ids.data();
}

// ─── Scene persistence (TOML round-trip) ─────────────────────────────────────

static std::string fmt_f(float v) {
    char buf[32];
    snprintf(buf, sizeof(buf), "%.6g", v);
    return buf;
}

// Emit a float array as a TOML inline array, e.g. [0.0, 1.0, 0.0]
static std::string fmt_farr(const float* arr, int n) {
    std::string s = "[";
    for (int i = 0; i < n; ++i) {
        if (i > 0) s += ", ";
        s += fmt_f(arr[i]);
    }
    s += "]";
    return s;
}

extern "C" void shadow_save_scene(const char* path) {
    if (path == nullptr) return;

    std::string out;
    out += "[scene]\n";
    out += "name    = \"Main\"\n";
    out += "version = \"1.0\"\n";
    out += "runtime = \"cpp23\"\n";

    for (const auto& e : g_entities) {
        out += "\n[[entity]]\n";
        out += "id   = \"" + (e.scene_id.empty() ? std::to_string(e.id) : e.scene_id) + "\"\n";
        out += "name = \"" + e.name + "\"\n";

        // Transform (every entity has one)
        auto* t = static_cast<TransformData*>(e.components[COMP_TRANSFORM]);
        if (t) {
            out += "\n  [[entity.component]]\n";
            out += "  type     = \"Transform\"\n";
            out += "  position = " + fmt_farr(t->position, 3) + "\n";
            out += "  rotation = " + fmt_farr(t->rotation, 4) + "\n";
            out += "  scale    = " + fmt_farr(t->scale, 3) + "\n";
        }

        // PlayerController
        auto* pc = static_cast<PlayerControllerData*>(e.components[COMP_PLAYER_CONTROLLER]);
        if (pc) {
            out += "\n  [[entity.component]]\n";
            out += "  type       = \"PlayerController\"\n";
            out += "  speed      = " + fmt_f(pc->speed) + "\n";
            out += "  jump_force = " + fmt_f(pc->jump_force) + "\n";
        }

        // Health
        auto* h = static_cast<HealthData*>(e.components[COMP_HEALTH]);
        if (h) {
            out += "\n  [[entity.component]]\n";
            out += "  type    = \"Health\"\n";
            out += "  current = " + fmt_f(h->current) + "\n";
            out += "  max     = " + fmt_f(h->max) + "\n";
            out += "  regen   = ";
            out += h->regen ? "true" : "false";
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

    // Clear existing scene state
    for (auto& e : g_entities) {
        detach_all_components(e);
    }
    g_entities.clear();
    g_next_entity_id = 1;

    // Simple line-by-line TOML parser for the subset we emit in save_scene.
    // Handles [[entity]] and [[entity.component]] sections with key = value lines.
    // Does not handle the full TOML spec — only the schema we produce.

    enum class Section { None, Scene, Entity, Component };

    Section    section        = Section::None;
    RuntimeEntity* cur_entity = nullptr;
    uint32_t   cur_comp_type  = COMP_COUNT; // invalid sentinel

    auto trim = [](const std::string& s) -> std::string {
        size_t a = s.find_first_not_of(" \t\r\n");
        if (a == std::string::npos) return "";
        size_t b = s.find_last_not_of(" \t\r\n");
        return s.substr(a, b - a + 1);
    };

    auto strip_quotes = [](const std::string& s) -> std::string {
        if (s.size() >= 2 && s.front() == '"' && s.back() == '"') {
            return s.substr(1, s.size() - 2);
        }
        return s;
    };

    // Parse a TOML inline float array "[a, b, c]" into out[0..n-1].
    auto parse_farr = [](const std::string& s, float* out, int n) {
        // Strip brackets
        size_t open  = s.find('[');
        size_t close = s.find(']');
        if (open == std::string::npos || close == std::string::npos) return;
        std::string inner = s.substr(open + 1, close - open - 1);
        std::istringstream ss(inner);
        std::string tok;
        int i = 0;
        while (std::getline(ss, tok, ',') && i < n) {
            // trim whitespace
            size_t a = tok.find_first_not_of(" \t");
            size_t b = tok.find_last_not_of(" \t");
            if (a != std::string::npos) {
                out[i++] = std::stof(tok.substr(a, b - a + 1));
            }
        }
    };

    std::string line;
    while (std::getline(file, line)) {
        std::string tl = trim(line);

        // Skip comments and blank lines
        if (tl.empty() || tl[0] == '#') continue;

        // Section headers
        if (tl == "[scene]") {
            section = Section::Scene;
            continue;
        }
        if (tl == "[[entity]]") {
            // Commit the previous in-flight entity (if any) — it was already
            // push_back'd when we first saw [[entity]], so nothing extra here.
            // Start a new entity accumulator.
            g_entities.emplace_back();
            cur_entity     = &g_entities.back();
            cur_entity->id = g_next_entity_id++;
            cur_comp_type  = COMP_COUNT;
            section        = Section::Entity;
            continue;
        }
        if (tl == "[[entity.component]]") {
            cur_comp_type = COMP_COUNT; // reset until we read "type ="
            section = Section::Component;
            continue;
        }

        // Key = value lines
        size_t eq = tl.find('=');
        if (eq == std::string::npos) continue;

        std::string key = trim(tl.substr(0, eq));
        std::string val = trim(tl.substr(eq + 1));

        if (section == Section::Entity && cur_entity != nullptr) {
            if (key == "id") {
                cur_entity->scene_id = strip_quotes(val);
            } else if (key == "name") {
                cur_entity->name = strip_quotes(val);
            }
        } else if (section == Section::Component && cur_entity != nullptr) {
            if (key == "type") {
                std::string comp_name = strip_quotes(val);
                if (comp_name == "Transform") {
                    cur_comp_type = COMP_TRANSFORM;
                    attach_component(*cur_entity, COMP_TRANSFORM);
                } else if (comp_name == "PlayerController") {
                    cur_comp_type = COMP_PLAYER_CONTROLLER;
                    attach_component(*cur_entity, COMP_PLAYER_CONTROLLER);
                } else if (comp_name == "Health") {
                    cur_comp_type = COMP_HEALTH;
                    attach_component(*cur_entity, COMP_HEALTH);
                } else {
                    cur_comp_type = COMP_COUNT;
                }
                continue;
            }

            // Component field values
            if (cur_comp_type == COMP_TRANSFORM) {
                auto* t = static_cast<TransformData*>(cur_entity->components[COMP_TRANSFORM]);
                if (!t) continue;
                if (key == "position") parse_farr(val, t->position, 3);
                else if (key == "rotation") parse_farr(val, t->rotation, 4);
                else if (key == "scale")    parse_farr(val, t->scale, 3);
            } else if (cur_comp_type == COMP_PLAYER_CONTROLLER) {
                auto* pc = static_cast<PlayerControllerData*>(cur_entity->components[COMP_PLAYER_CONTROLLER]);
                if (!pc) continue;
                if (key == "speed")           pc->speed      = std::stof(val);
                else if (key == "jump_force") pc->jump_force = std::stof(val);
            } else if (cur_comp_type == COMP_HEALTH) {
                auto* h = static_cast<HealthData*>(cur_entity->components[COMP_HEALTH]);
                if (!h) continue;
                if (key == "current")      h->current = std::stof(val);
                else if (key == "max")     h->max     = std::stof(val);
                else if (key == "regen")   h->regen   = (val == "true");
            }
        }
    }

    refresh_entity_cache();
}
