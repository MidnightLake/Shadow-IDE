#pragma once
/**
 * components.h — Example annotated game components for the hello_game runtime.
 * editor-header-tool parses this file to produce shadow_reflect.json.
 */
#include "shadow/shadow_reflect.h"

// ─── Transform ───────────────────────────────────────────────────────────────

SHADOW_COMPONENT()
struct TransformData {
    SHADOW_PROPERTY(float[3], "display_name=Position, step=0.1")
    float position[3] = {0.0f, 0.0f, 0.0f};

    SHADOW_PROPERTY(float[4], "display_name=Rotation (Quaternion), step=0.01, tooltip=XYZW quaternion")
    float rotation[4] = {0.0f, 0.0f, 0.0f, 1.0f};

    SHADOW_PROPERTY(float[3], "display_name=Scale, min=0.001, step=0.01")
    float scale[3] = {1.0f, 1.0f, 1.0f};
};

// ─── PlayerController ────────────────────────────────────────────────────────

SHADOW_COMPONENT()
struct PlayerControllerData {
    SHADOW_PROPERTY(float, "display_name=Move Speed, min=0, max=50, step=0.5")
    float speed = 5.0f;

    SHADOW_PROPERTY(float, "display_name=Jump Force, min=0, max=30, step=0.5")
    float jump_force = 8.0f;
};

// ─── Health ──────────────────────────────────────────────────────────────────

SHADOW_COMPONENT()
struct HealthData {
    SHADOW_PROPERTY(float, "display_name=Current HP, min=0, max=max, step=1")
    float current = 100.0f;

    SHADOW_PROPERTY(float, "display_name=Max HP, min=1, step=1")
    float max = 100.0f;

    SHADOW_PROPERTY(bool, "display_name=Regenerates")
    bool regen = false;
};
