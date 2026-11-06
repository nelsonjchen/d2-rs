// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
use super::{GoRng, OptimizationError, OptimizationWorkGuard, OwnerScoreSession, Point};
// sized_optimizer.go370/423: the same request guard covers candidate shuffling.
pub(super) fn shuffle(
    rng: &mut GoRng,
    points: &mut [Point],
    session: Option<&mut OwnerScoreSession<'_>>,
    guard: Option<&mut OptimizationWorkGuard<'_>>,
) -> Result<(), OptimizationError> {
    match (session, guard) {
        (Some(session), Some(guard)) => rng.shuffle_with_owner(points, session.owner_mut(), guard),
        (None, Some(guard)) => rng.shuffle_guarded(points, guard),
        (_, None) => {
            rng.shuffle(points);
            Ok(())
        }
    }
}
