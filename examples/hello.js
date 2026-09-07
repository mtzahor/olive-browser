// Run with: cargo run --locked --features js --bin olive-js -- examples/hello.js
const olives = ['Kalamata', 'Arbequina', 'Manzanilla'];
console.log('Olive JavaScript is running');
olives.map((name, index) => `${index + 1}. ${name}`).join(', ')
