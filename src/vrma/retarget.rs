mod bone;
mod expressions;

use crate::vrma::retarget::bone::VrmaRetargetingBonePlugin;
use crate::vrma::retarget::expressions::VrmaRetargetExpressionsPlugin;
use bevy::app::{App, Plugin, Update};
use bevy::prelude::*;
use bevy::window::RequestRedraw;

pub(crate) use expressions::VrmaExpressionNames;

#[derive(Component)]
pub(crate) struct CurrentRetargeting;

pub(super) struct VrmaRetargetPlugin;

impl Plugin for VrmaRetargetPlugin {
    fn build(
        &self,
        app: &mut App,
    ) {
        app.add_plugins((VrmaRetargetingBonePlugin, VrmaRetargetExpressionsPlugin))
            .add_systems(Update, request_redraw.run_if(playing_animation));
    }
}

fn playing_animation(
    changed_bones: Query<Entity, (Changed<Transform>, With<CurrentRetargeting>)>
) -> bool {
    !changed_bones.is_empty()
}

fn request_redraw(mut request: EventWriter<RequestRedraw>) {
    request.write(RequestRedraw);
}
