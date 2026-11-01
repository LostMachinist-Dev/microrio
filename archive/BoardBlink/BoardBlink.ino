void setup() {
  pinMode(LED_BLUE, OUTPUT);
  Serial.begin(9600);
  delay(1500);
}

void loop() {
  digitalWrite(LED_BLUE, HIGH);
  delay(1000);
  Serial.println("Im");
  digitalWrite(LED_BLUE, LOW);
  delay(10);
  Serial.println("alive");
}
