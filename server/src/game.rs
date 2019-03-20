use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant, SystemTime},
};
use tokyo::models::{
    BulletState, DeadPlayer, GameCommand, GameState, PlayerState, BULLET_RADIUS, BULLET_SPEED,
    PLAYER_BASE_SPEED, PLAYER_RADIUS,
};

const DEAD_PUNISH: Duration = Duration::from_secs(1);

pub const TICKS_PER_SECOND: f32 = 30.0;
const BOUNDS: (f32, f32) = (2880.0, 1920.0);
const MAX_CONCURRENT_BULLETS: usize = 4;

// Time until you start accruing points for surviving
const SURVIVAL_TIMEOUT: u64 = 10;

// Interval for accruing points after reaching the threshold
const SURVIVAL_POINT_INTERVAL: u64 = 4;

pub trait Triangle {
    fn x(&self) -> f32;
    fn y(&self) -> f32;
    fn angle(&self) -> f32;
    fn radius(&self) -> f32;

    fn is_colliding(&self, other: &Triangle) -> bool {
        let d_x = other.x() - self.x();
        let d_y = other.y() - self.y();
        let d_r = other.radius() + self.radius();
        let squared_dist = d_x * d_x + d_y * d_y;
        let squared_radii = d_r * d_r;

        squared_dist < squared_radii
    }
}

impl Triangle for PlayerState {
    fn x(&self) -> f32 {
        self.x
    }

    fn y(&self) -> f32 {
        self.y
    }

    fn angle(&self) -> f32 {
        self.angle
    }

    fn radius(&self) -> f32 {
        PLAYER_RADIUS
    }
}

impl Triangle for BulletState {
    fn x(&self) -> f32 {
        self.x
    }

    fn y(&self) -> f32 {
        self.y
    }

    fn angle(&self) -> f32 {
        self.angle
    }

    fn radius(&self) -> f32 {
        BULLET_RADIUS
    }
}

#[derive(Default, Debug)]
struct InternalPlayerState {
    last_fire_time: Option<Instant>,
}

pub struct Game {
    pub state: GameState,
    rng: rand::rngs::ThreadRng,
    bullet_id_counter: u32,
    // TODO: Merge into InternalPlayerState.
    survival_times: HashMap<u32, Instant>,
    started_at: Instant,
    internal_player_state_map: HashMap<u32, InternalPlayerState>,
}

impl Default for Game {
    fn default() -> Self {
        Self {
            state: GameState::new(BOUNDS),
            rng: Default::default(),
            bullet_id_counter: 0,
            survival_times: HashMap::new(),
            started_at: Instant::now(),
            internal_player_state_map: HashMap::new(),
        }
    }
}

impl Game {
    pub fn add_player(&mut self, player_id: u32) {
        let mut player = PlayerState::new(player_id);
        player.randomize(&mut self.rng, BOUNDS);
        self.state.players.push(player);
        self.survival_times
            .insert(player_id, Instant::now() + Duration::from_secs(SURVIVAL_TIMEOUT));
    }

    pub fn player_left(&mut self, player_id: u32) {
        info!("Player {} left!", player_id);

        if let Some(player) = self.state.players.iter_mut().find(|p| p.id == player_id) {
            player.throttle = 0.0;
        }

        self.survival_times.remove(&player_id);
    }

    pub fn handle_cmd(&mut self, player_id: u32, cmd: GameCommand) {
        // info!("Player {} sent command {:#?}", player_id, cmd);

        if let Some(player) = self.state.players.iter_mut().find(|p| p.id == player_id) {
            let internal_player_state = self
                .internal_player_state_map
                .entry(player.id)
                .or_insert(InternalPlayerState::default());
            match cmd {
                GameCommand::Rotate(angle) => {
                    player.angle = angle;
                },
                GameCommand::Throttle(throttle) => {
                    // Throttle ceiling can be at most 3.0 if you don't shoot.
                    let last_first_time =
                        internal_player_state.last_fire_time.unwrap_or(self.started_at);
                    let throttle_cap =
                        1.0 + ((Instant::now() - last_first_time).as_secs() as f32 / 60.0).min(2.0);

                    // Bound and re-map throttle inputs.
                    let throttle = throttle.max(0.0).min(throttle_cap);
                    player.throttle = throttle;
                },
                GameCommand::Fire => {
                    let active_bullets = self
                        .state
                        .bullets
                        .iter()
                        .filter(|bullet| bullet.player_id == player.id)
                        .count();

                    if active_bullets < MAX_CONCURRENT_BULLETS {
                        let bullet_id = self.bullet_id_counter;
                        self.bullet_id_counter += 1;

                        let distance_from_player: f32 = 5.0;
                        let (bullet_x, bullet_y) = angle_to_vector(player.angle);

                        self.state.bullets.push(BulletState {
                            id: bullet_id,
                            player_id: player.id,
                            angle: player.angle,
                            x: player.x + (bullet_x * distance_from_player),
                            y: player.y + (bullet_y * distance_from_player),
                        });
                        internal_player_state.last_fire_time = Some(Instant::now());
                    }
                },
            }
        }
    }

    pub fn init(&mut self) {}

    pub fn tick(&mut self, dt: f32) {
        // Revive the dead
        let now = SystemTime::now();
        let revived = self
            .state
            .dead
            .drain_filter(|corpse| corpse.respawn <= now)
            .map(|dead| dead.player)
            .map(|player| {
                println!("revived player {}", player.id);
                player
            });

        self.state.players.extend(revived);

        // Advance bullets
        for bullet in &mut self.state.bullets {
            let (vel_x, vel_y) = angle_to_vector(bullet.angle);

            bullet.x += vel_x * BULLET_SPEED * dt;
            bullet.y += vel_y * BULLET_SPEED * dt;
        }

        for player in &mut self.state.players {
            // Move the player
            let (vel_x, vel_y) = angle_to_vector(player.angle);

            player.x += vel_x * PLAYER_BASE_SPEED * player.throttle * dt;
            player.y += vel_y * PLAYER_BASE_SPEED * player.throttle * dt;

            // Keep the players in bounds
            player.x = player.x.max(PLAYER_RADIUS).min(BOUNDS.0 - PLAYER_RADIUS);
            player.y = player.y.max(PLAYER_RADIUS).min(BOUNDS.1 - PLAYER_RADIUS);
        }

        // Remove out-of-bound bullets
        self.state.bullets.retain(|b| {
            b.x > (BULLET_RADIUS)
                && b.x < (BOUNDS.0 + BULLET_RADIUS)
                && b.y > (BULLET_RADIUS)
                && b.y < (BOUNDS.1 + BULLET_RADIUS)
        });

        // count collisions
        let mut dead_players = HashSet::new();
        for player in &self.state.players {
            for other in &self.state.players {
                if player.id != other.id && player.is_colliding(other) {
                    dead_players.insert(player.id);
                    dead_players.insert(other.id);
                }
            }
        }

        for mut player in
            self.state.players.drain_filter(|player| dead_players.contains(&player.id))
        {
            player.randomize(&mut self.rng, BOUNDS);
            self.state.dead.push(DeadPlayer { respawn: SystemTime::now() + DEAD_PUNISH, player });
        }

        // count the dead
        let mut hits = vec![];
        let mut used_bullets = vec![];
        for bullet in &mut self.state.bullets {
            let deceased = self.state.players.drain_filter(|player| {
                if player.is_colliding(bullet) && bullet.player_id != player.id {
                    println!(
                        "Player {} killed player {} at ({}, {})",
                        bullet.player_id, player.id, bullet.x, bullet.y
                    );
                    hits.push(bullet.player_id);
                    used_bullets.push(bullet.id);

                    true
                } else {
                    false
                }
            });
            for mut player in deceased {
                // Reset their survival time bonus
                self.survival_times
                    .insert(player.id, Instant::now() + Duration::from_secs(SURVIVAL_TIMEOUT));

                player.randomize(&mut self.rng, BOUNDS);
                self.state
                    .dead
                    .push(DeadPlayer { respawn: SystemTime::now() + DEAD_PUNISH, player });
            }
        }

        // Clear out used bullets
        self.state.bullets.retain(|b| !used_bullets.contains(&b.id));

        // Update the scoreboard
        for player_id in hits {
            *self.state.scoreboard.entry(player_id).or_default() += 1;
        }

        // Reward players for staying alive
        for (player_id, next_reward_time) in &mut self.survival_times {
            if *next_reward_time <= Instant::now() {
                *self.state.scoreboard.entry(*player_id).or_default() += 1;

                *next_reward_time = Instant::now() + Duration::from_secs(SURVIVAL_POINT_INTERVAL);
            }
        }
    }
}

// TODO(jake): rewrite tests.... maybe

fn angle_to_vector(angle: f32) -> (f32, f32) {
    (angle.cos(), angle.sin())
}
