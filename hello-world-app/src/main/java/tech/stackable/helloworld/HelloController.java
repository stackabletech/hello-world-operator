package tech.stackable.helloworld;

import org.springframework.beans.factory.annotation.Value;
import org.springframework.web.bind.annotation.GetMapping;
import org.springframework.web.bind.annotation.RestController;

@RestController
public class HelloController {

	@Value("${greeting.recipient:World}")
	String recipient;

	@Value("${greeting.color:black}")
	String color;

	@GetMapping("/")
	public String index() {
		return "<h1 style=\"color:" + color +"\">Hello " + recipient + "!</h1>";
	}
}