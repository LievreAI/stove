use miniquad::*;
pub struct Cube {
    block: Pipeline,
    bindings: Bindings,
}

impl Cube {
    pub fn new(ctx: &mut Context) -> Self {
        let shader = Shader::new(
            ctx,
            include_str!("cube.vert"),
            include_str!("cube.frag"),
            ShaderMeta {
                uniforms: UniformBlockLayout {
                    uniforms: vec![
                        UniformDesc::new("model", UniformType::Mat4),
                        UniformDesc::new("view", UniformType::Mat4),
                        UniformDesc::new("tint", UniformType::Float3),
                    ],
                },
                images: vec![],
            },
        )
        .unwrap();
        Self {
            block: Pipeline::with_params(
                ctx,
                &[BufferLayout::default()],
                &[],
                shader,
                PipelineParams {
                    depth_test: Comparison::LessOrEqual,
                    depth_write: true,
                    primitive_type: PrimitiveType::Lines,
                    ..Default::default()
                },
            ),
            bindings: Bindings {
                vertex_buffers: vec![],
                index_buffer: Buffer::immutable(
                    ctx,
                    BufferType::IndexBuffer,
                    &[
                        1, 2, 0, 1, 2, 3, // front face
                        5, 6, 4, 5, 6, 7, // back face
                        3, 5, 1, 3, 5, 7, // top face
                        2, 4, 0, 2, 4, 6, // bottom face
                        0, 5, 4, 0, 5, 1, // left face
                        2, 7, 6, 2, 7, 3, // right face
                    ],
                ),
                images: vec![],
            },
        }
    }
    pub fn draw(&self, ctx: &mut Context, model: glam::Mat4, view: glam::Mat4, tint: glam::Vec3) {
        ctx.apply_pipeline(&self.block);
        ctx.apply_bindings(&self.bindings);
        ctx.apply_uniforms(&Uniforms { model, view, tint });
        ctx.draw(0, 36, 1);
    }
}

#[repr(C)]
struct Uniforms {
    model: glam::Mat4,
    view: glam::Mat4,
    tint: glam::Vec3,
}
