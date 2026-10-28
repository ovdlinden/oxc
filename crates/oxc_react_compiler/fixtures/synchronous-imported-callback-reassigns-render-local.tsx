import replace from 'string-replace-to-array';

function Component({text, pattern}) {
  let index = 0;
  return replace(text, pattern, match => <span key={index++}>{match}</span>);
}
