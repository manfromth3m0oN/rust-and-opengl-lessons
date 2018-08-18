use gl;
use failure;
use render_gl::{self, buffer, DebugLines};
use selection::{self, Selectables, SelectableAABB};
use resources::Resources;
use nalgebra as na;
use ncollide3d::bounding_volume::aabb::AABB;
use mesh;

mod dice_material_mesh {
    use render_gl::{data};

    #[derive(VertexAttribPointers)]
    #[derive(Copy, Clone, Debug)]
    #[repr(C, packed)]
    pub struct Vertex {
        #[location = "0"]
        pub pos: data::f32_f32_f32,
        #[location = "1"]
        pub uv: data::f16_f16,
        #[location = "2"]
        pub t: data::f32_f32_f32,
        #[location = "3"]
        pub n: data::f32_f32_f32,
    }
}

mod dice_material {
    use render_gl;
    use nalgebra as na;

    pub struct Material {
        texture_location: Option<i32>,
        texture_normals_location: Option<i32>,

        program_viewprojection_location: Option<i32>,
        program_model_location: Option<i32>,
        camera_pos_location: Option<i32>,
    }

    impl Material {
        pub fn load_for(program: &render_gl::Program) -> Material {
            Material {
                texture_location: program.get_uniform_location("Texture"),
                texture_normals_location: program.get_uniform_location("Normals"),

                program_viewprojection_location: program.get_uniform_location("ViewProjection"),
                program_model_location: program.get_uniform_location("Model"),
                camera_pos_location: program.get_uniform_location("CameraPos"),
            }
        }

        pub fn bind(&self, program: &render_gl::Program, viewprojection_matrix: &na::Matrix4<f32>, model_matrix: &na::Matrix4<f32>, camera_pos: &na::Vector3<f32>,
                    texture: &Option<render_gl::Texture>, texture_normals: &Option<render_gl::Texture>) {
            if let (Some(loc), &Some(ref texture)) = (self.texture_location, texture) {
                texture.bind_at(0);
                program.set_uniform_1i(loc, 0);
            }

            if let (Some(loc), &Some(ref texture)) = (self.texture_normals_location, texture_normals) {
                texture.bind_at(1);
                program.set_uniform_1i(loc, 1);
            }

            if let Some(loc) = self.program_viewprojection_location {
                program.set_uniform_matrix_4fv(loc, viewprojection_matrix);
            }
            if let Some(loc) = self.program_model_location {
                program.set_uniform_matrix_4fv(loc, model_matrix);
            }
            if let Some(loc) = self.camera_pos_location {
                program.set_uniform_3f(loc, camera_pos);
            }
        }
    }
}



pub struct Dice {
    transform: na::Isometry3<f32>,
    program: render_gl::Program,
    texture: Option<render_gl::Texture>,
    texture_normals: Option<render_gl::Texture>,
    material: dice_material::Material,
    _vbo: buffer::Buffer,
    _ebo: buffer::Buffer,
    index_count: i32,
    vao: buffer::VertexArray,
    debug_tangent_normals: render_gl::RayMarkers,
    selectable_aabb: Option<SelectableAABB>,
}

impl Dice {
    pub fn new(res: &Resources, gl: &gl::Gl, debug_lines: &DebugLines, selectables: &Selectables) -> Result<Dice, failure::Error> {

        // set up shader program

        let program = render_gl::Program::from_res(gl, res, "shaders/shiny")?;
        let p_material = dice_material::Material::load_for(&program);

        // this loader does not support file names with spaces
        let imported_models = res.load_obj("objs/dice.obj")?;

        // take first material in obj
        let material = imported_models.materials.into_iter().next();
        let material_index = material.as_ref().map(|_| 0); // it is first or None

        let texture = material.as_ref()
            .and_then(|m| m.diffuse_map.as_ref()
                .and_then(|resource_path|
                    render_gl::Texture::from_res_rgb(&resource_path)
                        .with_gen_mipmaps()
                        .load(gl, res)
                        .map_err(|e| println!("Error loading {}: {}", resource_path, e))
                        .ok()
                ));
        let texture_normals = material.as_ref()
            .and_then(|m| m.bump_map.as_ref()
                .and_then(|resource_path|
                    render_gl::Texture::from_res_rgb(&resource_path)
                        .with_gen_mipmaps()
                        .load(gl, res)
                        .map_err(|e| println!("Error loading {}: {}", resource_path, e))
                        .ok()
                ));

        // match mesh to material id and get the mesh
        let mesh = imported_models.meshes.into_iter()
            .filter(|model| model.material_index == material_index)
            .next()
            .expect("expected obj file to contain a mesh");

        let vbo_data = mesh.vertices.clone()
            .into_iter()
            .map(|v| {
                let tv = v.tangents.unwrap_or_else(|| {
                    println!("Missing tangent vectors");
                    mesh::Tangents::nans()
                });
                let uv = v.uv.unwrap_or_else(|| {
                    println!("Missing uv vectors");
                    [0.0, 0.0].into()
                });
                let normal = v.normal.unwrap_or_else(|| {
                    println!("Missing normal vectors");
                    [0.0, 0.0, 0.0].into()
                });
                dice_material_mesh::Vertex {
                    pos: (v.pos.x, v.pos.y, v.pos.z).into(),
                    uv: (uv.x, -uv.y).into(),
                    t: (tv.tangent.x, tv.tangent.y, tv.tangent.z).into(),
                    n: (normal.x, normal.y, normal.z).into(),
                }
            })
            .collect::<Vec<_>>();

        let ebo_data = mesh.triangle_indices();

        let vbo = buffer::Buffer::new_array(gl);
        vbo.bind();
        vbo.stream_draw_data(&vbo_data);
        vbo.unbind();

        let ebo = buffer::Buffer::new_element_array(gl);
        ebo.bind();
        ebo.stream_draw_data(&ebo_data);
        ebo.unbind();

        // set up vertex array object

        let vao = buffer::VertexArray::new(gl);

        vao.bind();
        vbo.bind();
        ebo.bind();
        dice_material_mesh::Vertex::vertex_attrib_pointers(gl);
        vao.unbind();

        vbo.unbind();
        ebo.unbind();

        let initial_isometry = na::Isometry3::identity();

        Ok(Dice {
            transform: initial_isometry,
            texture,
            texture_normals,
            program,
            material: p_material,
            _vbo: vbo,
            _ebo: ebo,
            index_count: ebo_data.len() as i32,
            vao,
            debug_tangent_normals: debug_lines.ray_markers(
                initial_isometry,
                vbo_data.iter().map(|v| (
                    na::Point3::new(v.pos.d0, v.pos.d1, v.pos.d2),
                    na::Vector3::new(v.n.d0, v.n.d1, v.n.d2) * 0.2,
                    na::Vector4::new(0.0, 0.0, 1.0, 1.0),
                )).chain(vbo_data.iter().map(|v| (
                    na::Point3::new(v.pos.d0, v.pos.d1, v.pos.d2),
                    na::Vector3::new(v.t.d0, v.t.d1, v.t.d2) * 0.2,
                    na::Vector4::new(0.0, 1.0, 0.0, 1.0),
                )))
            ),
            selectable_aabb: {
                let mut min_x = None;
                let mut min_y = None;
                let mut min_z = None;
                let mut max_x = None;
                let mut max_y = None;
                let mut max_z = None;

                fn update_min(val: &mut Option<f32>, new: f32) {
                    *val = match val {
                        None => Some(new),
                        Some(val) => if new < *val { Some(new) } else { return; },
                    };
                }

                fn update_max(val: &mut Option<f32>, new: f32) {
                    *val = match val {
                        None => Some(new),
                        Some(val) => if new > *val { Some(new) } else { return; },
                    };
                }

                for v in &vbo_data {
                    update_min(&mut min_x, v.pos.d0);
                    update_min(&mut min_y, v.pos.d1);
                    update_min(&mut min_z, v.pos.d2);
                    update_max(&mut max_x, v.pos.d0);
                    update_max(&mut max_y, v.pos.d1);
                    update_max(&mut max_z, v.pos.d2);
                }

                if let (Some(min_x), Some(min_y), Some(min_z), Some(max_x), Some(max_y), Some(max_z)) = (min_x, min_y, min_z, max_x, max_y, max_z) {
                    Some(
                        selectables.selectable(
                            AABB::new([min_x, min_y, min_z].into(), [max_x, max_y, max_z].into()),
                            initial_isometry,
                        )
                    )
                } else {
                    None
                }
            },
        })
    }

    pub fn update(&mut self, _delta: f32) {
        loop {
            let action = self.selectable_aabb.as_ref().and_then(|s| s.drain_pending_action());

            match action {
                Some(selection::Action::Click) => { self.selectable_aabb.as_ref().map(|s| s.select()); },
                Some(selection::Action::Drag { new_isometry }) => self.set_transform(new_isometry),
                _ => break,
            }
        }
    }

    pub fn set_transform(&mut self, isometry: na::Isometry3<f32>) {
        self.transform = isometry;
        if let Some(ref selectable) = self.selectable_aabb {
            selectable.update_isometry(isometry);
        }
        self.debug_tangent_normals.update_isometry(isometry);
    }

    pub fn render(&self, gl: &gl::Gl, viewprojection_matrix: &na::Matrix4<f32>, camera_pos: &na::Vector3<f32>) {
        self.program.set_used();

        self.material.bind(
            &self.program,
            viewprojection_matrix, &self.transform.to_homogeneous(), camera_pos,
            &self.texture, &self.texture_normals
        );
        self.vao.bind();

        unsafe {
            gl.DrawElements(
                gl::TRIANGLES, // mode
                self.index_count, // index vertex count
                gl::UNSIGNED_INT, // index type
                ::std::ptr::null(), // pointer to indices (we are using ebo configured at vao creation)
            );
        }

        self.vao.unbind();
    }
}