#version 330

flat in ivec2 selected;
flat in int id;

void main() {
    gl_FragColor = vec4(selected.x == 1 && selected.y == id ? 1 : 0, 1, 0.5, 1);
}