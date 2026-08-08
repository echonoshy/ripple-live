import type { QualityTier, VisualState } from './motion'

const STATE_INDEX: Record<VisualState, number> = {
  idle: 0,
  connecting: 1,
  listening: 2,
  thinking: 3,
  tool: 4,
  speaking: 5,
  ended: 6,
  error: 7,
}

const MAX_RENDER_DIMENSION = 8192
const MAX_PIXEL_RATIO = 2

const VERTEX_SHADER = `#version 300 es
void main() {
  vec2 p = vec2((gl_VertexID << 1) & 2, gl_VertexID & 2);
  gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}`

const FRAGMENT_SHADER = `#version 300 es
precision highp float;
uniform float uTime;
uniform float uInput;
uniform float uOutput;
uniform float uEnergy;
uniform float uGeometryEnergy;
uniform vec2 uResolution;
uniform int uState;
uniform int uQuality;
uniform int uReducedMotion;
out vec4 outColor;

float ball(vec2 p, vec2 c, float r) {
  return r * r / max(dot(p - c, p - c), 0.002);
}

void main() {
  vec2 p = (gl_FragCoord.xy * 2.0 - uResolution) / uResolution.y;
  float appearanceEnergy = clamp(uEnergy, 0.0, 1.0);
  float motion = uReducedMotion == 1 ? 0.0 : 1.0;
  float geometryEnergy = uReducedMotion == 1 ? 0.0 : uGeometryEnergy;
  float t = uTime * (0.42 + geometryEnergy * 1.2) * motion;
  float field = ball(p, vec2(0.0), 0.48 + geometryEnergy * 0.05);
  field += ball(p, vec2(cos(t), sin(t)) * 0.22, 0.25);
  field += ball(p, vec2(cos(t + 2.1), sin(t + 2.1)) * 0.20, 0.23);
  field += ball(p, vec2(cos(t + 4.2), sin(t + 4.2)) * 0.21, 0.24);
  // High quality adds fine field detail; low quality evaluates only the core body.
  if (uQuality == 1) {
    field += ball(p, vec2(cos(t * 0.7 + 1.0), sin(t * 0.7 + 1.0)) * 0.27, 0.15);
    field += ball(p, vec2(cos(t * 0.8 + 3.5), sin(t * 0.8 + 3.5)) * 0.26, 0.14);
  }
  float body = smoothstep(1.35, 1.65, field);
  float edge = smoothstep(1.05, 1.45, field) - body;
  float highlight = exp(-8.0 * dot(p - vec2(-0.18, 0.22), p - vec2(-0.18, 0.22)));
  vec3 deep = vec3(0.063, 0.231, 0.38);
  vec3 mid = vec3(0.298, 0.659, 0.886);
  vec3 ice = vec3(0.88, 0.97, 1.0);
  float brightness = mix(0.94, 1.06, appearanceEnergy);
  float highlightStrength = mix(0.78, 0.88, appearanceEnergy);
  float edgeIntensity = mix(0.40, 0.48, appearanceEnergy);
  float edgeAlpha = mix(0.50, 0.58, appearanceEnergy);
  vec3 color = mix(deep, mid, clamp(field - 1.2, 0.0, 1.0));
  color = mix(color, ice, highlight * body * highlightStrength);
  outColor = vec4(
    (color * body + mid * edge * edgeIntensity) * brightness,
    body + edge * edgeAlpha
  );
}`

export type OrbFrame = {
  state: VisualState
  inputLevel: number
  outputLevel: number
  reducedMotion: boolean
  qualityTier: QualityTier
  nowMs: number
}

export type OrbRenderer = {
  update(frame: OrbFrame): void
  resize(width: number, height: number, pixelRatio: number): void
  dispose(): void
}

function clamp(value: number, minimum: number, maximum: number) {
  if (!Number.isFinite(value)) return minimum
  return Math.min(maximum, Math.max(minimum, value))
}

function frameEnergy(frame: OrbFrame) {
  switch (frame.state) {
    case 'idle': return 0.08
    case 'connecting': return 0.18
    case 'listening': return 0.08 + clamp(frame.inputLevel, 0, 1) * 0.92
    case 'thinking': return 0.24
    case 'tool': return 0.14
    case 'speaking': return 0.08 + clamp(frame.outputLevel, 0, 1) * 0.92
    case 'ended': return 0.04
    case 'error': return 0.12
  }
}

export function createOrbRenderer(canvas: HTMLCanvasElement): OrbRenderer {
  const gl = canvas.getContext('webgl2', { alpha: true, antialias: false })
  if (!gl) throw new Error('webgl2_unavailable')

  let vertexShader: WebGLShader | null = null
  let fragmentShader: WebGLShader | null = null
  let program: WebGLProgram | null = null
  let vao: WebGLVertexArrayObject | null = null

  const compile = (kind: number, source: string) => {
    const shader = gl.createShader(kind)
    if (!shader) throw new Error('shader_create_failed')
    gl.shaderSource(shader, source)
    gl.compileShader(shader)
    if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
      const message = gl.getShaderInfoLog(shader) ?? 'shader_compile_failed'
      gl.deleteShader(shader)
      throw new Error(message)
    }
    return shader
  }

  try {
    vertexShader = compile(gl.VERTEX_SHADER, VERTEX_SHADER)
    fragmentShader = compile(gl.FRAGMENT_SHADER, FRAGMENT_SHADER)
    program = gl.createProgram()
    if (!program) throw new Error('program_create_failed')
    gl.attachShader(program, vertexShader)
    gl.attachShader(program, fragmentShader)
    gl.linkProgram(program)
    if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
      throw new Error(gl.getProgramInfoLog(program) ?? 'program_link_failed')
    }

    gl.useProgram(program)
    vao = gl.createVertexArray()
    if (!vao) throw new Error('vertex_array_create_failed')
    gl.bindVertexArray(vao)

    const uniforms = {
      time: gl.getUniformLocation(program, 'uTime'),
      input: gl.getUniformLocation(program, 'uInput'),
      output: gl.getUniformLocation(program, 'uOutput'),
      energy: gl.getUniformLocation(program, 'uEnergy'),
      geometryEnergy: gl.getUniformLocation(program, 'uGeometryEnergy'),
      state: gl.getUniformLocation(program, 'uState'),
      quality: gl.getUniformLocation(program, 'uQuality'),
      reducedMotion: gl.getUniformLocation(program, 'uReducedMotion'),
      resolution: gl.getUniformLocation(program, 'uResolution'),
    }

    gl.detachShader(program, vertexShader)
    gl.detachShader(program, fragmentShader)
    gl.deleteShader(vertexShader)
    gl.deleteShader(fragmentShader)
    vertexShader = null
    fragmentShader = null

    let disposed = false
    const resize = (width: number, height: number, pixelRatio: number) => {
      if (disposed) return
      const ratio = clamp(pixelRatio, 1, MAX_PIXEL_RATIO)
      canvas.width = Math.round(
        clamp(width, 1, MAX_RENDER_DIMENSION / ratio) * ratio,
      )
      canvas.height = Math.round(
        clamp(height, 1, MAX_RENDER_DIMENSION / ratio) * ratio,
      )
      gl.viewport(0, 0, canvas.width, canvas.height)
      gl.uniform2f(uniforms.resolution, canvas.width, canvas.height)
    }

    resize(canvas.clientWidth, canvas.clientHeight, 1)

    const update = (frame: OrbFrame) => {
      if (disposed) return
      const energy = frameEnergy(frame)
      gl.uniform1f(
        uniforms.time,
        frame.reducedMotion
          ? 0
          : clamp(frame.nowMs, 0, Number.MAX_SAFE_INTEGER) / 1000,
      )
      gl.uniform1f(uniforms.input, clamp(frame.inputLevel, 0, 1))
      gl.uniform1f(uniforms.output, clamp(frame.outputLevel, 0, 1))
      gl.uniform1f(uniforms.energy, energy)
      gl.uniform1f(uniforms.geometryEnergy, frame.reducedMotion ? 0 : energy)
      gl.uniform1i(uniforms.state, STATE_INDEX[frame.state])
      gl.uniform1i(uniforms.quality, frame.qualityTier === 'high' ? 1 : 0)
      gl.uniform1i(uniforms.reducedMotion, frame.reducedMotion ? 1 : 0)
      gl.drawArrays(gl.TRIANGLES, 0, 3)
    }

    const dispose = () => {
      if (disposed) return
      disposed = true
      gl.bindVertexArray(null)
      gl.useProgram(null)
      gl.deleteVertexArray(vao)
      gl.deleteProgram(program)
    }

    return { update, resize, dispose }
  } catch (error) {
    if (vao) gl.deleteVertexArray(vao)
    if (program) gl.deleteProgram(program)
    if (vertexShader) gl.deleteShader(vertexShader)
    if (fragmentShader) gl.deleteShader(fragmentShader)
    throw error
  }
}
