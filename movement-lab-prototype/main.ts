import {
  addBot,
  command,
  ghostStates,
  holdModifierState,
  onFrame,
  onWorld,
  type Player,
  type Steering,
  toast,
  writeField,
} from 'modlock'

interface RecordedStep {
  tick: bigint
  steering: Steering
}

interface Route {
  hero: number
  team: number
  firstTick: bigint
  steps: RecordedStep[]
}

interface Recording {
  hero: number
  team: number
  steps: RecordedStep[]
}

interface Playback {
  owner: Player
  bot: Player
  route: Route
  index: number
  startedAt?: bigint
  lastServerTick?: bigint
  styledEntity?: number
}

const recordings = new Map<number, Recording>()
const routes = new Map<number, Route>()
const playbacks = new Map<number, Playback>()

function stopPlayback(ownerSlot: number): void {
  const playback = playbacks.get(ownerSlot)
  if (!playback) return

  playback.bot.steer()
  playback.bot.removeBot()
  playbacks.delete(ownerSlot)
}

function styleGhost(playback: Playback): void {
  const pawn = playback.bot.pawn()
  if (!pawn || playback.styledEntity === pawn.entity) return

  playback.styledEntity = pawn.entity
  for (const state of ghostStates) {
    holdModifierState(pawn.entity, state, true)
  }

  // Same translucent look used by Modlock's Puppet helper.
  writeField(pawn.entity, 'CBaseModelEntity', 'm_nRenderMode', 'uint8', 1)
  writeField(
    pawn.entity,
    'CBaseModelEntity',
    'm_clrRender',
    'uint32',
    0xffffff | (110 << 24),
  )
}

command('splitrec', (player) => {
  const pawn = player.pawn()
  if (!pawn) {
    toast(player, 'SPLIT: no hero to record.')
    return
  }

  stopPlayback(player.slot)
  recordings.set(player.slot, {
    hero: pawn.hero,
    team: pawn.team,
    steps: [],
  })

  player.watchMovement(true)
  toast(player, 'SPLIT: recording started.')
})

command('splitstop', (player) => {
  const recording = recordings.get(player.slot)
  if (!recording) {
    toast(player, 'SPLIT: no recording is running.')
    return
  }

  player.watchMovement(false)
  recordings.delete(player.slot)

  if (recording.steps.length < 2) {
    toast(player, 'SPLIT: recording was too short.')
    return
  }

  routes.set(player.slot, {
    hero: recording.hero,
    team: recording.team,
    firstTick: recording.steps[0].tick,
    steps: recording.steps,
  })

  toast(player, `SPLIT: saved ${recording.steps.length} movement ticks.`)
})

command('splitghost', (player) => {
  const route = routes.get(player.slot)
  if (!route) {
    toast(player, 'SPLIT: record a route first with /splitrec.')
    return
  }

  stopPlayback(player.slot)

  const first = route.steps[0].steering
  const bot = addBot({
    name: 'SPLIT Ghost',
    team: route.team,
    hero: route.hero,
    position: first.position,
  })

  if (!bot) {
    toast(player, 'SPLIT: could not spawn the ghost.')
    return
  }

  playbacks.set(player.slot, {
    owner: player,
    bot,
    route,
    index: 0,
  })

  toast(player, 'SPLIT: ghost ready.')
})

command('splitclear', (player) => {
  player.watchMovement(false)
  recordings.delete(player.slot)
  routes.delete(player.slot)
  stopPlayback(player.slot)
  toast(player, 'SPLIT: route cleared.')
})

onWorld(() => {
  for (const slot of playbacks.keys()) stopPlayback(slot)
  recordings.clear()
  routes.clear()
})

onFrame((frame) => {
  // Record the exact movement command + resulting pose for every watched tick.
  for (const sample of frame.movement) {
    const recording = recordings.get(sample.player.slot)
    if (!recording || !sample.command) continue

    const pawn = sample.player.pawn()
    if (!pawn || pawn.entity !== sample.pawn) continue

    recording.steps.push({
      tick: frame.tick,
      steering: {
        command: sample.command,
        position: sample.position,
        velocity: sample.velocity,
        grounded: sample.grounded,
        facing: {
          pitch: pawn.eyeAngles.pitch,
          yaw: pawn.eyeAngles.yaw,
          roll: pawn.eyeAngles.roll,
        },
      },
    })
  }

  // Replay one recorded command on each corresponding server tick.
  for (const [ownerSlot, playback] of playbacks) {
    styleGhost(playback)

    if (!playback.bot.pawn()) continue
    if (playback.lastServerTick === frame.tick) continue
    playback.lastServerTick = frame.tick

    if (playback.startedAt === undefined) {
      playback.startedAt = frame.tick
    }

    const elapsed = frame.tick - playback.startedAt

    while (playback.index < playback.route.steps.length) {
      const step = playback.route.steps[playback.index]
      const dueAt = step.tick - playback.route.firstTick
      if (dueAt > elapsed) break

      playback.bot.steer(step.steering)
      playback.index += 1
    }

    if (playback.index >= playback.route.steps.length) {
      playback.bot.steer()
      toast(playback.owner, 'SPLIT: ghost finished.')
      playbacks.delete(ownerSlot)
    }
  }
})
