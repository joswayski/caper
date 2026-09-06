import {
  AdditiveBlending,
  BufferGeometry,
  DynamicDrawUsage,
  Float32BufferAttribute,
  Group,
  LineBasicMaterial,
  LineLoop,
  Points,
  PointsMaterial,
} from "three";

const ringColors = [0xb64d32, 0x637a43, 0xd7a08f];

/** A quiet, animated voice signal that wraps around the chat model. */
export function createVoiceField() {
  const group = new Group();
  const rings = ringColors.map((color, ringIndex) => {
    const geometry = new BufferGeometry();
    const position = new Float32BufferAttribute(180 * 3, 3);
    const positions = position.array as Float32Array;
    position.setUsage(DynamicDrawUsage);
    geometry.setAttribute("position", position);
    const material = new LineBasicMaterial({
      color,
      transparent: true,
      opacity: 0.52 - ringIndex * 0.09,
      depthTest: false,
      depthWrite: false,
      blending: AdditiveBlending,
    });
    const line = new LineLoop(geometry, material);
    line.renderOrder = -1;
    group.add(line);
    return { positions, position, geometry, material };
  });

  const particleCount = 54;
  const particleColors = new Float32Array(particleCount * 3);
  for (let i = 0; i < particleCount; i++) {
    const color = ringColors[i % ringColors.length];
    particleColors[i * 3] = ((color >> 16) & 255) / 255;
    particleColors[i * 3 + 1] = ((color >> 8) & 255) / 255;
    particleColors[i * 3 + 2] = (color & 255) / 255;
  }
  const particleGeometry = new BufferGeometry();
  const particlePosition = new Float32BufferAttribute(particleCount * 3, 3);
  const liveParticlePositions = particlePosition.array as Float32Array;
  particlePosition.setUsage(DynamicDrawUsage);
  particleGeometry.setAttribute("position", particlePosition);
  particleGeometry.setAttribute("color", new Float32BufferAttribute(particleColors, 3));
  const particleMaterial = new PointsMaterial({
    size: 0.085,
    vertexColors: true,
    transparent: true,
    opacity: 0.62,
    depthTest: false,
    depthWrite: false,
    blending: AdditiveBlending,
  });
  const particles = new Points(particleGeometry, particleMaterial);
  particles.renderOrder = -1;
  group.add(particles);

  const update = (elapsed: number, still = false) => {
    const time = still ? 0 : elapsed;
    rings.forEach(({ positions, position }, ringIndex) => {
      for (let i = 0; i < 180; i++) {
        const angle = i / 180 * Math.PI * 2;
        const envelope = 0.07
          + Math.max(0, Math.sin(angle * 3 - time * 0.0013 + ringIndex * 1.9)) ** 2 * 0.2;
        const signal = Math.sin(angle * (18 + ringIndex * 3) - time * 0.0032 + ringIndex) * envelope;
        const radius = ringIndex * 0.17 + signal;
        positions[i * 3] = Math.cos(angle) * (6.48 + radius);
        positions[i * 3 + 1] = Math.sin(angle) * (4.38 + radius * 0.7);
        positions[i * 3 + 2] = -0.28 - ringIndex * 0.18
          + Math.sin(angle * 2 + time * 0.00045 + ringIndex) * 0.12;
      }
      position.needsUpdate = true;
    });

    for (let i = 0; i < particleCount; i++) {
      const angle = i / particleCount * Math.PI * 2 + Math.sin(i * 12.9898) * 0.035;
      const drift = Math.sin(time * (0.00028 + i % 5 * 0.000035) + i * 1.7) * 0.16;
      const radius = 0.34 + (i % 7) * 0.045 + drift;
      liveParticlePositions[i * 3] = Math.cos(angle) * (6.48 + radius);
      liveParticlePositions[i * 3 + 1] = Math.sin(angle) * (4.38 + radius * 0.72);
      liveParticlePositions[i * 3 + 2] = -0.18 - (i % 9) * 0.055;
    }
    particlePosition.needsUpdate = true;
    particleMaterial.opacity = still ? 0.5 : 0.56 + Math.sin(time * 0.0013) * 0.1;
  };

  update(0, true);

  return {
    group,
    update,
    dispose() {
      rings.forEach(({ geometry, material }) => {
        geometry.dispose();
        material.dispose();
      });
      particleGeometry.dispose();
      particleMaterial.dispose();
    },
  };
}
