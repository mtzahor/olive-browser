document.getElementById('status').textContent = 'External JavaScript is ready.';
function pickOlive() {
    harvest++;
    document.getElementById('count').textContent = harvest + ' olives picked';
    document.title = 'Olive — harvest ' + harvest;
    console.log('Picked', harvest);
}
console.log('External script loaded once.');
