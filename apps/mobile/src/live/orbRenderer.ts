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
uniform float uRippleProgress;
uniform float uRippleAlpha;
uniform float uHaloPulse;
uniform vec2 uResolution;
uniform int uState;
uniform int uQuality;
uniform int uReducedMotion;
out vec4 outColor;

float hash(vec2 p) {
  return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453123);
}

float noise(vec2 p) {
  vec2 cell = floor(p);
  vec2 local = fract(p);
  vec2 blend = local * local * (3.0 - 2.0 * local);
  return mix(
    mix(hash(cell), hash(cell + vec2(1.0, 0.0)), blend.x),
    mix(hash(cell + vec2(0.0, 1.0)), hash(cell + vec2(1.0)), blend.x),
    blend.y
  );
}

float fbm(vec2 p) {
  float value = 0.0;
  float amplitude = 0.5;
  mat2 rotation = mat2(0.80, -0.60, 0.60, 0.80);
  for (int octave = 0; octave < 5; octave++) {
    if (uQuality == 0 && octave >= 3) break;
    value += noise(p) * amplitude;
    p = rotation * p * 2.03 + vec2(13.7, 7.9);
    amplitude *= 0.5;
  }
  return value;
}

void main() {
  vec2 uv = (gl_FragCoord.xy * 2.0 - uResolution) / uResolution.y;
  float radius = 0.52;
  float distanceToCore = length(uv);
  float coreMask = 1.0 - smoothstep(radius - 0.012, radius + 0.008, distanceToCore);
  float appearanceEnergy = clamp(uEnergy, 0.0, 1.0);
  float geometryEnergy = uReducedMotion == 1 ? 0.0 : uGeometryEnergy;
  float slowTime = uReducedMotion == 1 ? 0.0 : uTime;
  float stateSpeed = uState == 3 ? 1.04 : (uState == 5 ? 1.08 : 1.0);
  slowTime *= mix(0.88, 1.12, geometryEnergy) * stateSpeed;
  float cloud = fbm(uv * 2.7 + vec2(slowTime * 0.07, -slowTime * 0.05));
  float ribbon = fbm(uv * 4.1 + vec2(-slowTime * 0.11, slowTime * 0.08));
  float highlight = exp(-10.0 * dot(uv - vec2(-0.18, 0.22), uv - vec2(-0.18, 0.22)));
  float dawnReflection = (1.0 - smoothstep(-0.44, 0.08, uv.y))
    * (1.0 - smoothstep(0.12, radius, distanceToCore))
    * smoothstep(0.48, 0.82, ribbon);
  vec3 deep = vec3(0.039, 0.180, 0.459);
  vec3 cobalt = vec3(0.184, 0.467, 0.902);
  vec3 softBlue = vec3(0.608, 0.765, 1.0);
  vec3 cream = vec3(1.0, 0.965, 0.914);
  vec3 dawn = mix(vec3(1.0, 0.898, 0.863), vec3(0.753, 0.788, 1.0), 0.45);
  vec3 color = mix(deep, cobalt, smoothstep(0.22, 0.82, cloud));
  color = mix(color, softBlue, smoothstep(0.58, 0.92, ribbon) * 0.42);
  color = mix(color, cream, highlight * 0.58);
  color = mix(color, dawn, dawnReflection * 0.08);
  float brightness = mix(0.94, 1.06, appearanceEnergy);
  float stateBrightness = uState == 1 ? 0.94 : (uState == 5 ? 1.04 : 1.0);

  float halo = exp(-52.0 * pow(max(distanceToCore - radius, 0.0), 2.0));
  float p = clamp(uRippleProgress, 0.0, 1.0);
  float eased = 1.0 - pow(1.0 - p, 3.0);
  float ringRadius = radius * mix(1.03, 1.28, eased);
  float ringWidth = mix(0.010, 0.038, p);
  float ring = exp(-pow((distanceToCore - ringRadius) / ringWidth, 2.0));
  float haloAlpha = mix(0.04, 0.06, clamp(uEnergy + uHaloPulse, 0.0, 1.0));
  float ringAlpha = uRippleProgress < 0.0 ? 0.0 : ring * uRippleAlpha;

  float outerHaloAlpha = halo * haloAlpha * (1.0 - coreMask);
  float alpha = clamp(coreMask + outerHaloAlpha + ringAlpha, 0.0, 1.0);
  vec3 premultipliedColor = clamp(color * brightness * stateBrightness, 0.0, 1.0) * coreMask;
  premultipliedColor += softBlue * outerHaloAlpha;
  premultipliedColor += cream * ringAlpha;
  outColor = vec4(premultipliedColor, alpha);
}`

export type OrbFrame = {
  state: VisualState
  inputLevel: number
  outputLevel: number
  reducedMotion: boolean
  qualityTier: QualityTier
  rippleProgress: number | null
  rippleAlpha: number
  haloPulse: number
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
      rippleProgress: gl.getUniformLocation(program, 'uRippleProgress'),
      rippleAlpha: gl.getUniformLocation(program, 'uRippleAlpha'),
      haloPulse: gl.getUniformLocation(program, 'uHaloPulse'),
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
      gl.uniform1f(
        uniforms.rippleProgress,
        frame.reducedMotion || frame.rippleProgress === null
          ? -1
          : clamp(frame.rippleProgress, 0, 1),
      )
      gl.uniform1f(
        uniforms.rippleAlpha,
        frame.reducedMotion ? 0 : clamp(frame.rippleAlpha, 0, 1),
      )
      gl.uniform1f(uniforms.haloPulse, clamp(frame.haloPulse, 0, 1))
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
