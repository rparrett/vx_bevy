use std::cell::RefCell;

use super::{
    chunks::{ChunkEntities, ChunkLoadingStage, ChunkUpdateEvent},
    Chunk, ChunkShape, Voxel, CHUNK_LENGTH,
};
use crate::{
    utils::ThreadLocalRes,
    voxel::{
        render::{mesh_buffer, MeshBuffers},
        storage::VoxelMap,
    },
};
use bevy::{
    prelude::*,
    render::{primitives::Aabb, render_resource::PrimitiveTopology},
    tasks::ComputeTaskPool,
};

/// Attaches to the newly inserted chunk entities components required for rendering.
pub fn prepare_chunks(
    chunks: Query<(Entity, &Chunk), Added<Chunk>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut cmds: Commands,
) {
    for (chunk, chunk_key) in chunks.iter() {
        cmds.entity(chunk)
            .insert_bundle(PbrBundle {
                mesh: meshes.add(shape::Box::new(16.0, 16.0, 16.0).into()),
                transform: Transform::from_translation(chunk_key.0.location().as_vec3()),
                ..Default::default()
            })
            .insert(Aabb::from_min_max(
                Vec3::ZERO,
                Vec3::new(
                    CHUNK_LENGTH as f32,
                    CHUNK_LENGTH as f32,
                    CHUNK_LENGTH as f32,
                ),
            ))
            .insert(NeedsMeshing);
    }
}

/// Marks chunk entities that need meshing by attaching them a [`NeedsMeshing`] marker component.
fn queue_meshing(
    mut updates: EventReader<ChunkUpdateEvent>,
    mut cmds: Commands,
    chunk_entities: Res<ChunkEntities>,
) {
    for update in updates.iter() {
        if let Some(entity) = chunk_entities.entity(update.0) {
            cmds.entity(entity).insert(NeedsMeshing);
        }
    }
}

//todo: filter meshing order so that chunks which are closer to the camera get meshed first.
//perf: reuse buffers between frames.
fn mesh_chunks(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut chunk_query: QuerySet<(
        QueryState<(&Chunk, Entity), With<NeedsMeshing>>,
        QueryState<&Handle<Mesh>, With<NeedsMeshing>>,
    )>,
    mesh_buffers: Local<ThreadLocalRes<RefCell<MeshBuffers<Voxel, ChunkShape>>>>,
    chunks: Res<VoxelMap<Voxel, ChunkShape>>,
    frame_budget: Res<WorldChunksMeshingFrameBudget>,
    task_pool: Res<ComputeTaskPool>,
) {
    let generated_meshes = task_pool.scope(|scope| {
        chunk_query
            .q0()
            .iter()
            .take(frame_budget.meshes_per_frame)
            .map(|(chunk, entity)| (entity, chunks.buffer_at(chunk.0).unwrap())) //safe to unwrap since chunk data is guaranted to exist.
            .map(|(entity, buffer)| {
                //because resources aren't static, futures must be spawned locally.
                let mesh_buffers_handle = mesh_buffers.get_handle();
                scope.spawn_local(async move {
                    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList);
                    let mut mesh_buffers = &mut mesh_buffers_handle
                        .get_or(|| {
                            RefCell::new(MeshBuffers::<Voxel, ChunkShape>::new(ChunkShape {}))
                        })
                        .borrow_mut();

                    mesh_buffer(buffer, &mut mesh_buffers, &mut mesh, 1.0);

                    (entity, mesh)
                })
            })
            .collect()
    });

    for (entity, mesh) in generated_meshes {
        *meshes
            .get_mut(chunk_query.q1().get(entity).unwrap())
            .unwrap() = mesh;

        commands.entity(entity).remove::<NeedsMeshing>();
    }
}

/// Label for the stage housing the chunk rendering systems.
#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Debug, Hash, StageLabel)]
pub struct ChunkRenderingStage;

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Debug, Hash, SystemLabel)]
pub enum ChunkRenderingSystem {
    /// Attaches to the newly inserted chunk entities components required for rendering.
    Prepare,

    /// Marks chunk entities that need meshing.
    QueueMeshing,

    /// Mesh actual chunks
    MeshChunks,
}

/// Handles the rendering of the chunks.
pub struct VoxelWorldRenderingPlugin;

pub struct WorldChunksMeshingFrameBudget {
    pub meshes_per_frame: usize,
}

impl Plugin for VoxelWorldRenderingPlugin {
    fn build(&self, app: &mut bevy::prelude::App) {
        app.add_stage_after(
            ChunkLoadingStage,
            ChunkRenderingStage,
            SystemStage::parallel()
                .with_system(prepare_chunks.label(ChunkRenderingSystem::Prepare))
                .with_system(
                    queue_meshing
                        .label(ChunkRenderingSystem::QueueMeshing)
                        .after(ChunkRenderingSystem::Prepare),
                )
                .with_system(
                    mesh_chunks
                        .label(ChunkRenderingSystem::MeshChunks)
                        .after(ChunkRenderingSystem::QueueMeshing),
                ),
        )
        .insert_resource(WorldChunksMeshingFrameBudget {
            meshes_per_frame: 16,
        });
    }
}

/// A component marking that a chunk needs meshing.
#[derive(Component)]
struct NeedsMeshing;
