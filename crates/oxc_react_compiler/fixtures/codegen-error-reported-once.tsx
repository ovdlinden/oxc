function AssistantMessage({message, live}) {
  const streaming = message.time.completed === undefined;
  const onlyReasoning =
    !streaming &&
    !message.content.some(
      part => part.type === 'tool' || (part.type === 'text' && part.text),
    );
  const texts = partTexts(live, message, 'text');
  const reasonings = partTexts(live, message, 'reasoning');
  const ordinals = {text: 0, reasoning: 0};

  return (
    <div>
      {message.content.map((part, index) => {
        const key = `${message.id}-${index}`;
        switch (part.type) {
          case 'text': {
            const text = texts[ordinals.text++] ?? '';
            return text ? <Markdown key={key} text={text} /> : null;
          }
          case 'reasoning': {
            const text = reasonings[ordinals.reasoning++] ?? '';
            return text ? <Reasoning key={key} text={text} open={onlyReasoning} /> : null;
          }
          case 'tool':
            return <ToolCard key={key} part={part} />;
        }
      })}
    </div>
  );
}
