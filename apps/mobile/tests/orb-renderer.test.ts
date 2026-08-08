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
  nowMs,
})

test('reduced motion freezes geometry uniforms while brightness energy remains live', () => {
  const { gl, renderer } = createHarness()

  renderer.update(reducedFrame(0.1, 1000))
  renderer.update(reducedFrame(0.9, 2000))

  assert.deepEqual(gl.floatUniforms.get('uTime'), [0, 0])
  assert.deepEqual(gl.floatUniforms.get('uGeometryEnergy'), [0, 0])
  const energy = gl.floatUniforms.get('uEnergy')
  assert.ok(energy)
  assert.notEqual(energy[0], energy[1])
})

test('fragment shader consumes clamped energy only through appearance output', () => {
  const { gl } = createHarness()

  assert.match(
    gl.fragmentSource,
    /float appearanceEnergy = clamp\(uEnergy, 0\.0, 1\.0\);/,
  )
  assert.match(
    gl.fragmentSource,
    /float brightness = mix\(0\.94, 1\.06, appearanceEnergy\);/,
  )
  assert.match(
    gl.fragmentSource,
    /outColor = vec4\([^;]*brightness,[^;]*edgeAlpha\s*\);/,
  )

  const geometry = gl.fragmentSource.slice(
    gl.fragmentSource.indexOf('float geometryEnergy'),
    gl.fragmentSource.indexOf('float body'),
  )
  assert.doesNotMatch(geometry, /\buEnergy\b|\bappearanceEnergy\b/)
})
