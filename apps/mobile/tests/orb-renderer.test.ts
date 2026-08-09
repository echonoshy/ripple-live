import assert from 'node:assert/strict'
import test from 'node:test'
import { createOrbRenderer, type OrbFrame } from '../src/live/orbRenderer.ts'

type UniformLocation = WebGLUniformLocation & { name: string }
type FakeShader = WebGLShader & { kind: number }

class FakeWebGl {
  readonly VERTEX_SHADER = 0x8b31
  readonly FRAGMENT_SHADER = 0x8b30
  readonly COMPILE_STATUS = 0x8b81
  readonly LINK_STATUS = 0x8b82
  readonly TRIANGLES = 0x0004
  readonly floatUniforms = new Map<string, number[]>()
  fragmentSource = ''

  createShader(kind: number) { return { kind } as FakeShader }
  shaderSource(shader: FakeShader, source: string) {
    if (shader.kind === this.FRAGMENT_SHADER) this.fragmentSource = source
  }
  compileShader() {}
  getShaderParameter() { return true }
  getShaderInfoLog() { return null }
  deleteShader() {}
  createProgram() { return {} as WebGLProgram }
  attachShader() {}
  linkProgram() {}
  getProgramParameter() { return true }
  getProgramInfoLog() { return null }
  useProgram() {}
  createVertexArray() { return {} as WebGLVertexArrayObject }
  bindVertexArray() {}
  getUniformLocation(_program: WebGLProgram, name: string) {
    return { name } as UniformLocation
  }
  detachShader() {}
  viewport() {}
  uniform2f() {}
  uniform1i() {}
  drawArrays() {}
  deleteVertexArray() {}
  deleteProgram() {}

  uniform1f(location: UniformLocation | null, value: number) {
    if (!location) return
    const values = this.floatUniforms.get(location.name) ?? []
    values.push(value)
    this.floatUniforms.set(location.name, values)
  }
}

function createHarness() {
  const gl = new FakeWebGl()
  const canvas = {
    clientWidth: 200,
    clientHeight: 200,
    width: 0,
    height: 0,
    getContext: () => gl,
  } as unknown as HTMLCanvasElement
  return { gl, renderer: createOrbRenderer(canvas) }
}

const reducedFrame = (inputLevel: number, nowMs: number): OrbFrame => ({
  state: 'listening',
  inputLevel,
  outputLevel: 0,
  reducedMotion: true,
  qualityTier: 'low',
  rippleProgress: null,
  rippleAlpha: 0,
  haloPulse: 0,
  nowMs,
})

test('reduced motion freezes geometry uniforms while brightness energy remains live', () => {
  const { gl, renderer } = createHarness()

  renderer.update(reducedFrame(0.1, 1000))
  renderer.update(reducedFrame(0.9, 2000))

  assert.deepEqual(gl.floatUniforms.get('uTime'), [0, 0])
  assert.deepEqual(gl.floatUniforms.get('uGeometryEnergy'), [0, 0])
  assert.deepEqual(gl.floatUniforms.get('uRippleProgress'), [-1, -1])
  const energy = gl.floatUniforms.get('uEnergy')
  assert.ok(energy)
  assert.notEqual(energy[0], energy[1])
})

test('fragment shader uses a domain-warped volumetric fluid material', () => {
  const { gl } = createHarness()

  assert.match(gl.fragmentSource, /uniform float uRippleProgress;/)
  assert.match(gl.fragmentSource, /uniform float uRippleAlpha;/)
  assert.match(gl.fragmentSource, /uniform float uHaloPulse;/)
  assert.match(gl.fragmentSource, /float radius = 0\.76;/)
  assert.match(
    gl.fragmentSource,
    /float coreMask = 1\.0 - smoothstep\(radius - 0\.010, radius \+ 0\.006, distanceToCore\);/,
  )
  assert.match(gl.fragmentSource, /float liveDrive =[^;]*max\(uInput, uOutput\)/)
  assert.match(gl.fragmentSource, /vec2 domainWarp = vec2\(/)
  assert.match(gl.fragmentSource, /float whiteMass = smoothstep\(/)
  assert.match(gl.fragmentSource, /float cyanMass = smoothstep\(/)
  assert.match(gl.fragmentSource, /float bluePocket = smoothstep\(/)
  assert.match(gl.fragmentSource, /vec3 deepBlue = vec3\(0\.015, 0\.185, 0\.780\);/)
  assert.match(gl.fragmentSource, /vec3 clearCyan = vec3\(0\.360, 0\.890, 1\.000\);/)
  assert.match(gl.fragmentSource, /vec3 pearlWhite = vec3\(0\.965, 1\.000, 0\.985\);/)
  assert.match(gl.fragmentSource, /float sphereDepth = sqrt\(/)
  assert.match(gl.fragmentSource, /float fresnel = pow\(/)
  assert.doesNotMatch(gl.fragmentSource, /float surface = flowUv\.y/)
  assert.doesNotMatch(gl.fragmentSource, /float ball\(/)
})

test('keeps listening motion calm and dampens microphone-driven warping', () => {
  const { gl } = createHarness()

  assert.doesNotMatch(gl.fragmentSource, /stateSpeed|uniform int uState/)
  assert.match(gl.fragmentSource, /liveDrive \* 0\.045/)
})

test('integrates motion time continuously across state changes', () => {
  const { gl, renderer } = createHarness()

  renderer.update({
    ...reducedFrame(0.2, 1000),
    reducedMotion: false,
    state: 'listening',
  })
  renderer.update({
    ...reducedFrame(0.2, 1016),
    reducedMotion: false,
    state: 'thinking',
  })

  const times = gl.floatUniforms.get('uTime') ?? []
  assert.equal(times.length, 2)
  assert.ok(times[1] > times[0])
  assert.ok(times[1] - times[0] < 0.02)
})

test('rotates the orb slowly in one direction without oscillating', () => {
  const { gl } = createHarness()

  assert.match(gl.fragmentSource, /float rotationAngle = slowTime \* 0\.080;/)
  assert.match(gl.fragmentSource, /vec2 flowUv = flowRotation \* sphereUv;/)
  assert.doesNotMatch(gl.fragmentSource, /float rotationAngle\s*=\s*[^;\n]*sin\(/)
})

test('fragment shader composites one eased outward ring and near halo', () => {
  const { gl } = createHarness()

  assert.match(gl.fragmentSource, /float halo = exp\(-110\.0 \* pow\(max\(distanceToCore - radius, 0\.0\), 2\.0\)\);/)
  assert.match(gl.fragmentSource, /float eased = 1\.0 - pow\(1\.0 - p, 3\.0\);/)
  assert.match(gl.fragmentSource, /float ringRadius = radius \* mix\(1\.03, 1\.28, eased\);/)
  assert.match(gl.fragmentSource, /float ringWidth = mix\(0\.010, 0\.038, p\);/)
  assert.match(gl.fragmentSource, /float ringAlpha = uRippleProgress < 0\.0 \? 0\.0 : ring \* uRippleAlpha;/)
})

test('renderer forwards ripple and halo frame values to the single shader pass', () => {
  const { gl, renderer } = createHarness()

  renderer.update({
    ...reducedFrame(0.4, 1000),
    reducedMotion: false,
    rippleProgress: 0.35,
    rippleAlpha: 0.08,
    haloPulse: 0.6,
  })

  assert.deepEqual(gl.floatUniforms.get('uRippleProgress'), [0.35])
  assert.deepEqual(gl.floatUniforms.get('uRippleAlpha'), [0.08])
  assert.deepEqual(gl.floatUniforms.get('uHaloPulse'), [0.6])
})

test('fragment shader keeps energy-driven changes inside the fixed silhouette', () => {
  const { gl } = createHarness()

  assert.match(
    gl.fragmentSource,
    /float appearanceEnergy = clamp\(uEnergy, 0\.0, 1\.0\);/,
  )
  assert.match(
    gl.fragmentSource,
    /float brightness = mix\(0\.98, 1\.10, appearanceEnergy\);/,
  )

  const silhouette = gl.fragmentSource.slice(
    gl.fragmentSource.indexOf('float radius'),
    gl.fragmentSource.indexOf('float appearanceEnergy'),
  )
  assert.doesNotMatch(silhouette, /\buEnergy\b|\bgeometryEnergy\b|\bappearanceEnergy\b/)
})
