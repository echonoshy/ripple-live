import type { QualityTier, VisualState } from './motion'

const STATE_SPEED: Record<VisualState, number> = {
  idle: 0.52,
  connecting: 0.38,
  listening: 0.54,
  thinking: 0.66,
  tool: 0.60,
  speaking: 0.70,
  ended: 0.30,
  error: 0.24,
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
  float radius = 0.76;
  float distanceToCore = length(uv);
  float coreMask = 1.0 - smoothstep(radius - 0.010, radius + 0.006, distanceToCore);
  float appearanceEnergy = clamp(uEnergy, 0.0, 1.0);
  float geometryEnergy = uReducedMotion == 1 ? 0.0 : uGeometryEnergy;
  float liveDrive = uReducedMotion == 1 ? 0.0 : clamp(max(uInput, uOutput), 0.0, 1.0);
  float voiceDirection = uReducedMotion == 1 ? 0.0 : clamp(uOutput - uInput, -1.0, 1.0);
  float slowTime = uReducedMotion == 1 ? 0.0 : uTime;

  vec2 sphereUv = uv / radius;
  float sphereDepth = sqrt(max(0.0, 1.0 - dot(sphereUv, sphereUv)));
  float rotationAngle = slowTime * 0.080;
  mat2 flowRotation = mat2(
    cos(rotationAngle), -sin(rotationAngle),
    sin(rotationAngle), cos(rotationAngle)
  );
  vec2 flowUv = flowRotation * sphereUv;
  vec2 drift = vec2(
    slowTime * (0.080 + 0.025 * voiceDirection),
    -slowTime * (0.052 - 0.018 * voiceDirection)
  );
  vec2 domainWarp = vec2(
    fbm(flowUv * 1.34 + drift + vec2(2.4, 5.1)),
    fbm(flowUv * 1.47 - drift * 0.76 + vec2(7.2, 1.6))
  ) - 0.5;
  flowUv += domainWarp * mix(0.38, 0.64, geometryEnergy);
  flowUv += vec2(
    sin(slowTime * 1.7 + sphereUv.y * 2.2),
    cos(slowTime * 1.3 - sphereUv.x * 2.0)
  ) * liveDrive * 0.045;

  float cloudA = fbm(flowUv * 1.38 + drift * 0.42 + vec2(0.8, 4.6));
  float cloudB = fbm(flowUv * 1.82 - drift * 0.58 + vec2(6.7, 2.3));
  float cloudC = fbm(flowUv * 2.46 + domainWarp * 1.15 + vec2(3.4, 8.1));
  float ribbon = 0.5 + 0.5 * sin(
    flowUv.x * 2.55
    - flowUv.y * 1.35
    + cloudA * 4.4
    + slowTime * 0.36
  );
  float whiteMass = smoothstep(
    0.50,
    0.78,
    cloudA * 0.58 + cloudB * 0.24 + ribbon * 0.18
  );
  float cyanMass = smoothstep(
    0.30,
    0.72,
    cloudB * 0.58 + cloudC * 0.28 + (1.0 - ribbon) * 0.14
  );
  float bluePocket = smoothstep(
    0.43,
    0.78,
    cloudC * 0.66 + (1.0 - cloudA) * 0.34
  );

  vec3 deepBlue = vec3(0.015, 0.185, 0.780);
  vec3 electricBlue = vec3(0.025, 0.405, 1.000);
  vec3 clearCyan = vec3(0.360, 0.890, 1.000);
  vec3 pearlWhite = vec3(0.965, 1.000, 0.985);
  vec3 color = mix(deepBlue, electricBlue, 0.48 + cloudB * 0.42);
  color = mix(color, clearCyan, cyanMass * 0.76);
  color = mix(color, electricBlue, bluePocket * 0.42);
  color = mix(color, pearlWhite, whiteMass * 0.92);

  vec3 normal = normalize(vec3(sphereUv, sphereDepth));
  vec3 lightDirection = normalize(vec3(-0.36, 0.52, 0.78));
  float diffuse = 0.80 + 0.20 * max(dot(normal, lightDirection), 0.0);
  float fresnel = pow(1.0 - sphereDepth, 2.2);
  float specular = pow(max(dot(normal, lightDirection), 0.0), 18.0);
  color *= diffuse;
  color += pearlWhite * specular * (0.10 + whiteMass * 0.10);
  color += clearCyan * fresnel * 0.16;
  float brightness = mix(0.98, 1.10, appearanceEnergy);

  float halo = exp(-110.0 * pow(max(distanceToCore - radius, 0.0), 2.0));
  float p = clamp(uRippleProgress, 0.0, 1.0);
  float eased = 1.0 - pow(1.0 - p, 3.0);
  float ringRadius = radius * mix(1.03, 1.28, eased);
  float ringWidth = mix(0.010, 0.038, p);
  float ring = exp(-pow((distanceToCore - ringRadius) / ringWidth, 2.0));
  float haloAlpha = mix(0.01, 0.025, clamp(uEnergy + uHaloPulse, 0.0, 1.0));
  float ringAlpha = uRippleProgress < 0.0 ? 0.0 : ring * uRippleAlpha;

  float outerHaloAlpha = halo * haloAlpha * (1.0 - coreMask);
  float alpha = clamp(coreMask + outerHaloAlpha + ringAlpha, 0.0, 1.0);
  vec3 premultipliedColor = clamp(color * brightness, 0.0, 1.0) * coreMask;
  premultipliedColor += clearCyan * outerHaloAlpha;
  premultipliedColor += pearlWhite * ringAlpha;
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
    case 'listening': return 0.08 + clamp(frame.inputLevel, 0, 1) * 0.64
    case 'thinking': return 0.24
    case 'tool': return 0.14
    case 'speaking': return 0.08 + clamp(frame.outputLevel, 0, 1) * 0.72
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

    let lastNowMs: number | null = null
    let motionTime = 0
    let motionSpeed = STATE_SPEED.idle
    let visualEnergy = 0.08

    const update = (frame: OrbFrame) => {
      if (disposed) return
      const nowMs = clamp(frame.nowMs, 0, Number.MAX_SAFE_INTEGER)
      const elapsedMs = lastNowMs === null
        ? 1000 / 60
        : clamp(nowMs - lastNowMs, 0, 100)
      lastNowMs = nowMs
      const speedBlend = 1 - Math.exp(-elapsedMs / 700)
      const energyBlend = 1 - Math.exp(-elapsedMs / 420)
      const targetEnergy = frameEnergy(frame)
      motionSpeed += (STATE_SPEED[frame.state] - motionSpeed) * speedBlend
      visualEnergy += (targetEnergy - visualEnergy) * energyBlend
      if (!frame.reducedMotion) {
        const energySpeed = 0.72 + visualEnergy * 0.20
        motionTime += elapsedMs / 1000 * motionSpeed * energySpeed
      }
      gl.uniform1f(
        uniforms.time,
        frame.reducedMotion ? 0 : motionTime,
      )
      gl.uniform1f(uniforms.input, clamp(frame.inputLevel, 0, 1))
      gl.uniform1f(uniforms.output, clamp(frame.outputLevel, 0, 1))
      gl.uniform1f(uniforms.energy, visualEnergy)
      gl.uniform1f(uniforms.geometryEnergy, frame.reducedMotion ? 0 : visualEnergy)
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
