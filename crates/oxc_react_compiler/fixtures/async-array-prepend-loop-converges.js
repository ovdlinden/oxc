function Component({items}) {
  let count = 0;
  const handleClick = () => count++;
  let values = [];
  for (const item of items) {
    values = [item, ...values];
  }
  return <div>{values.length}</div>;
}
