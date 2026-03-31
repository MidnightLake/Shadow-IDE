use bytemuck::{Pod, Zeroable};
use editor_core::ViewMode;
use glam::{Mat4, Vec2, Vec3};

#[derive(Debug, Clone)]
pub struct OrbitCamera {
    pub target: Vec3,
    pub distance: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub fov_y: f32,
    pub near: f32,
    pub far: f32,
}

impl Default for OrbitCamera {
    fn default() -> Self {
        Self {
            target: Vec3::ZERO,
            distance: 12.0,
            yaw: std::f32::consts::FRAC_PI_4,
            pitch: 0.4,
            fov_y: 60.0_f32.to_radians(),
            near: 0.1,
            far: 1000.0,
        }
    }
}

impl OrbitCamera {
    pub fn eye_position(&self) -> Vec3 {
        let x = self.distance * self.pitch.cos() * self.yaw.sin();
        let y = self.distance * self.pitch.sin();
        let z = self.distance * self.pitch.cos() * self.yaw.cos();
        self.target + Vec3::new(x, y, z)
    }

    pub fn view_matrix(&self) -> Mat4 {
        Mat4::look_at_rh(self.eye_position(), self.target, Vec3::Y)
    }

    pub fn projection_matrix(&self, aspect: f32, mode: ViewMode) -> Mat4 {
        match mode {
            ViewMode::Perspective3D => {
                Mat4::perspective_rh(self.fov_y, aspect, self.near, self.far)
            }
            ViewMode::Orthographic2D => {
                let half_h = self.distance * 0.5;
                let half_w = half_h * aspect;
                Mat4::orthographic_rh(-half_w, half_w, -half_h, half_h, self.near, self.far)
            }
        }
    }

    pub fn view_proj(&self, aspect: f32, mode: ViewMode) -> Mat4 {
        self.projection_matrix(aspect, mode) * self.view_matrix()
    }

    pub fn rotate(&mut self, delta_yaw: f32, delta_pitch: f32) {
        self.yaw += delta_yaw;
        self.pitch = (self.pitch + delta_pitch).clamp(
            -std::f32::consts::FRAC_PI_2 + 0.01,
            std::f32::consts::FRAC_PI_2 - 0.01,
        );
    }

    pub fn zoom(&mut self, delta: f32) {
        self.distance = (self.distance - delta).clamp(0.5, 500.0);
    }

    pub fn pan(&mut self, delta: Vec2) {
        let view = self.view_matrix();
        let right = Vec3::new(view.col(0).x, view.col(1).x, view.col(2).x);
        let up = Vec3::new(view.col(0).y, view.col(1).y, view.col(2).y);
        let speed = self.distance * 0.002;
        self.target += right * (-delta.x * speed) + up * (delta.y * speed);
    }

    pub fn uniforms(&self, aspect: f32, mode: ViewMode) -> CameraUniforms {
        let view = self.view_matrix();
        let view_proj = self.view_proj(aspect, mode);
        let eye = self.eye_position();
        CameraUniforms {
            view_proj: view_proj.to_cols_array_2d(),
            view: view.to_cols_array_2d(),
            camera_pos: [eye.x, eye.y, eye.z, 1.0],
        }
    }
}

#[repr(C)]
#[derive(Debug, Copy, Clone, Pod, Zeroable)]
pub struct CameraUniforms {
    pub view_proj: [[f32; 4]; 4],
    pub view: [[f32; 4]; 4],
    pub camera_pos: [f32; 4],
}
