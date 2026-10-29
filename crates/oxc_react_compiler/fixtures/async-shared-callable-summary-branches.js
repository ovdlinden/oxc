function Component({flag}) {
  let count = 0;
  const handleClick = () => count++;
  const run = async () => {
    await 0;
  };
  const first = () => {
    if (flag) run();
    else run();
  };
  const second = () => {
    if (flag) first();
    else first();
  };
  const third = () => {
    if (flag) second();
    else second();
  };
  const fourth = () => {
    if (flag) third();
    else third();
  };
  const fifth = () => {
    if (flag) fourth();
    else fourth();
  };
  const sixth = () => {
    if (flag) fifth();
    else fifth();
  };
  const seventh = () => {
    if (flag) sixth();
    else sixth();
  };
  const eighth = () => {
    if (flag) seventh();
    else seventh();
  };
  const ninth = () => {
    if (flag) eighth();
    else eighth();
  };
  const tenth = () => {
    if (flag) ninth();
    else ninth();
  };
  const eleventh = () => {
    if (flag) tenth();
    else tenth();
  };
  const twelfth = () => {
    if (flag) eleventh();
    else eleventh();
  };
  const thirteenth = () => {
    if (flag) twelfth();
    else twelfth();
  };
  const fourteenth = () => {
    if (flag) thirteenth();
    else thirteenth();
  };
  fourteenth();
  return null;
}
